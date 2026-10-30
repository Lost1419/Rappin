//! Bounded concurrent stdio transport over the shared CLI operation catalog.
use super::*;
use std::collections::HashMap;
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader},
    sync::mpsc,
    task::{AbortHandle, Id, JoinSet},
};

struct Job {
    id: Value,
    abort: AbortHandle,
    write: bool,
    cancelled: bool,
}
fn response(id: Value, result: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"result":result})
}
fn error(id: Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}
async fn send(sender: &mpsc::Sender<Value>, value: Value) -> Result<()> {
    sender
        .send(value)
        .await
        .map_err(|_| YappinError::Config("MCP output closed".into()))
}
async fn frame<R: AsyncBufRead + Unpin>(reader: &mut R) -> std::io::Result<Option<Vec<u8>>> {
    let mut result = Vec::new();
    let mut oversized = false;
    loop {
        let chunk = reader.fill_buf().await?;
        if chunk.is_empty() {
            return Ok((!result.is_empty() || oversized).then_some(result));
        }
        let end = chunk.iter().position(|byte| *byte == b'\n');
        let len = end.map_or(chunk.len(), |n| n + 1);
        if !oversized {
            if result.len() + len > 262144 {
                oversized = true;
                result.clear();
            } else {
                result.extend_from_slice(&chunk[..len]);
            }
        }
        reader.consume(len);
        if end.is_some() {
            return Ok(Some(result));
        }
    }
}
async fn execute(request: Value, context: Context, cursor: Option<PathBuf>) -> Value {
    let name = request["params"]["name"].as_str().unwrap_or("");
    let empty = json!({});
    let args = request["params"].get("arguments").unwrap_or(&empty);
    let result = async {
        let (args, ctx) = agent_catalog::prepare(name, args, &context, cursor.as_deref())?;
        run(args, &ctx).await
    }
    .await;
    let (failed, data) = match result {
        Ok(data) => (false, json!({"ok":true,"data":data})),
        Err(error) => (
            true,
            json!({"ok":false,"error":error_json(&error,!agent_catalog::is_write(name))}),
        ),
    };
    let data = redact(data, context.token.as_deref());
    json!({"isError":failed,"structuredContent":data,"content":[{"type":"text","text":data.to_string()}]})
}
fn immediate(request: &Value, id: &Value) -> Option<Value> {
    match request["method"].as_str() {
        Some("initialize") => {
            let requested = request["params"]["protocolVersion"]
                .as_str()
                .unwrap_or("2025-06-18");
            let version = if matches!(requested, "2025-06-18" | "2025-11-25") {
                requested
            } else {
                "2025-06-18"
            };
            Some(response(
                id.clone(),
                json!({"protocolVersion":version,"capabilities":{"tools":{}},"serverInfo":{"name":"rappin","version":env!("CARGO_PKG_VERSION")}}),
            ))
        }
        Some("ping") => Some(response(id.clone(), json!({}))),
        Some("tools/list") => Some(response(
            id.clone(),
            json!({"tools":agent_catalog::tools()}),
        )),
        Some("tools/call") => None,
        _ => Some(error(id.clone(), -32601, "Method not found")),
    }
}
async fn dispatch(
    request: Value,
    ctx: &Context,
    cursor: Option<&std::path::Path>,
    tasks: &mut JoinSet<Value>,
    jobs: &mut HashMap<Id, Job>,
    sender: &mpsc::Sender<Value>,
) -> Result<()> {
    if request["jsonrpc"] != "2.0" || !request["method"].is_string() {
        return send(
            sender,
            error(
                request.get("id").cloned().unwrap_or(Value::Null),
                -32600,
                "Invalid Request",
            ),
        )
        .await;
    }
    if request["method"] == "notifications/cancelled" {
        if let Some(id) = request["params"].get("requestId") {
            for job in jobs.values_mut().filter(|job| &job.id == id && !job.write) {
                job.cancelled = true;
                job.abort.abort();
            }
        }
        return Ok(());
    }
    let Some(id) = request.get("id").cloned() else {
        return Ok(());
    };
    if !id.is_string() && id.as_i64().is_none() && id.as_u64().is_none() {
        return send(
            sender,
            error(
                Value::Null,
                -32600,
                "Request ID must be a string or integer",
            ),
        )
        .await;
    }
    if jobs.values().any(|job| job.id == id) {
        return send(sender, error(id, -32600, "Request ID is already in use")).await;
    }
    if let Some(value) = immediate(&request, &id) {
        return send(sender, value).await;
    }
    if tasks.len() >= 16 {
        return send(
            sender,
            error(
                id,
                -32000,
                "Too many concurrent tool calls; retry after an active call completes",
            ),
        )
        .await;
    }
    let write = agent_catalog::is_write(request["params"]["name"].as_str().unwrap_or(""));
    let abort = tasks.spawn(execute(
        request,
        ctx.clone(),
        cursor.map(std::path::Path::to_path_buf),
    ));
    jobs.insert(
        abort.id(),
        Job {
            id,
            abort,
            write,
            cancelled: false,
        },
    );
    Ok(())
}
pub async fn serve(ctx: &Context, cursor: Option<&std::path::Path>) -> Result<()> {
    let (sender, mut receiver) = mpsc::channel::<Value>(32);
    let writer = tokio::spawn(async move {
        let mut stdout = tokio::io::stdout();
        while let Some(value) = receiver.recv().await {
            stdout.write_all(format!("{value}\n").as_bytes()).await?;
            stdout.flush().await?;
        }
        Ok::<_, std::io::Error>(())
    });
    // A dedicated reader retains partial frames while tool tasks finish.
    let (input_sender, mut input) = mpsc::channel(16);
    let reader_task = tokio::spawn(async move {
        let mut reader = BufReader::new(tokio::io::stdin());
        loop {
            match frame(&mut reader).await {
                Ok(Some(line)) => {
                    if input_sender.send(Ok(line)).await.is_err() {
                        break;
                    }
                }
                Ok(None) => break,
                Err(error) => {
                    let _ = input_sender.send(Err(error)).await;
                    break;
                }
            }
        }
    });
    let mut tasks = JoinSet::new();
    let mut jobs: HashMap<Id, Job> = HashMap::new();
    let mut eof = false;
    while !eof || !tasks.is_empty() {
        tokio::select! {
            finished=tasks.join_next_with_id(),if !tasks.is_empty()=>{
                match finished {
                    Some(Ok((task_id,result))) => {
                        if let Some(job)=jobs.remove(&task_id) {
                            if !job.cancelled { send(&sender,response(job.id,result)).await?; }
                        }
                    }
                    Some(Err(failure)) => {
                        if let Some(job)=jobs.remove(&failure.id()) {
                            if !job.cancelled { send(&sender,error(job.id,-32603,"tool execution failed")).await?; }
                        }
                    }
                    None => (),
                }
            }
            line=input.recv(),if !eof=>{
                if let Some(line)=line {
                    match serde_json::from_slice(&line?) {
                        Ok(request)=>dispatch(request,ctx,cursor,&mut tasks,&mut jobs,&sender).await?,
                        Err(_)=>send(&sender,error(Value::Null,-32700,"Parse error or frame exceeds 256 KiB")).await?,
                    }
                } else { eof=true; }
            }
        }
    }
    drop(sender);
    reader_task.abort();
    writer
        .await
        .map_err(|_| YappinError::Config("MCP writer stopped".into()))??;
    Ok(())
}
