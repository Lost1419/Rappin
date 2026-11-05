use super::protocol::{self, CogEvent, Listener, Manifest, Wire};
use crate::{
    BotHandle, Client, Cog, CogButton, CogCommand, Command, InteractionContext, MessageContext,
    Result, YappinError,
};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command as Process},
    sync::{Mutex, RwLock},
};

#[path = "watch.rs"]
mod watch;
use watch::fingerprint;

/// A separately compiled Rust binary (or example) containing WorkerCog::run().
#[derive(Clone, Debug)]
pub struct WorkerProject {
    manifest: PathBuf,
    target: String,
    example: bool,
    features: Vec<String>,
    package: Option<String>,
    offline: bool,
    watch_paths: Vec<PathBuf>,
    pub build_timeout: Duration,
    pub startup_timeout: Duration,
    pub handler_timeout: Duration,
}
impl WorkerProject {
    pub fn new(manifest: impl Into<PathBuf>, binary: impl Into<String>) -> Self {
        Self {
            manifest: manifest.into(),
            target: binary.into(),
            example: false,
            features: vec![],
            package: None,
            offline: false,
            watch_paths: vec![],
            build_timeout: Duration::from_secs(300),
            startup_timeout: Duration::from_secs(10),
            handler_timeout: Duration::from_secs(120),
        }
    }
    pub fn example(manifest: impl Into<PathBuf>, name: impl Into<String>) -> Self {
        let mut spec = Self::new(manifest, name);
        spec.example = true;
        spec
    }
    pub fn features(mut self, features: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.features = features.into_iter().map(Into::into).collect();
        self
    }
    pub fn package(mut self, package: impl Into<String>) -> Self {
        self.package = Some(package.into());
        self
    }
    pub fn offline(mut self, offline: bool) -> Self {
        self.offline = offline;
        self
    }
    /// Additional required source/config roots, including roots outside the
    /// manifest directory or inside normally skipped directories.
    pub fn watch_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.watch_paths.push(path.into());
        self
    }
    async fn build(&self) -> Result<PathBuf> {
        use super::diagnostics::{error, CompilerReport};
        use crate::ReloadPhase::Compilation;
        let manifest = tokio::fs::canonicalize(&self.manifest).await.map_err(|e| {
            error(
                Compilation,
                "cannot inspect worker manifest",
                Some(&e.into()),
                false,
                false,
                None,
            )
        })?;
        let root = manifest.parent().ok_or_else(|| {
            error(
                Compilation,
                "manifest has no parent",
                None,
                false,
                false,
                None,
            )
        })?;
        let mut report = CompilerReport::new(root);
        let mut cargo = Process::new("cargo");
        environment(&mut cargo, true);
        cargo
            .current_dir(root)
            .args(["build", "--message-format=json", "--manifest-path"])
            .arg(&manifest)
            .arg(if self.example { "--example" } else { "--bin" })
            .arg(&self.target)
            .arg("--target-dir")
            .arg(root.join("target/rappin-hot"))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if !self.features.is_empty() {
            cargo.arg("--features").arg(self.features.join(","));
        }
        if let Some(package) = &self.package {
            cargo.arg("--package").arg(package);
        }
        if self.offline {
            cargo.arg("--offline");
        }
        let mut child = cargo.spawn().map_err(|e| {
            error(
                Compilation,
                "cannot start Cargo",
                Some(&e.into()),
                false,
                false,
                None,
            )
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            error(
                Compilation,
                "Cargo stdout unavailable",
                None,
                false,
                false,
                None,
            )
        })?;
        let mut stderr = child.stderr.take().ok_or_else(|| {
            error(
                Compilation,
                "Cargo stderr unavailable",
                None,
                false,
                false,
                None,
            )
        })?;
        let mut reader = BufReader::new(stdout);
        let build = async {
            let output = async {
                let mut executable = None;
                loop {
                    let mut line = Vec::new();
                    let n = (&mut reader)
                        .take(1024 * 1024 + 1)
                        .read_until(b'\n', &mut line)
                        .await?;
                    if n == 0 {
                        break;
                    }
                    if line.len() > 1024 * 1024 {
                        report.failure.truncated = true;
                        return Err(YappinError::Config("compiler frame exceeds 1 MiB".into()));
                    }
                    if let Ok(value) = serde_json::from_slice::<Value>(&line) {
                        if value["reason"] == "compiler-artifact"
                            && value["target"]["name"] == self.target
                        {
                            if let Some(path) = value["executable"].as_str() {
                                executable = Some(PathBuf::from(path));
                            }
                        }
                        if value["reason"] == "compiler-message" {
                            report.push(&value["message"]);
                        }
                    }
                }
                Ok(executable)
            };
            // Drain concurrently with bounded memory. Raw stderr can include paths,
            // arbitrary build-script output or secrets, so it is not forwarded.
            let drain = async {
                tokio::io::copy(&mut stderr, &mut tokio::io::sink())
                    .await
                    .map(|_| ())
                    .map_err(YappinError::from)
            };
            let (executable, _) = tokio::try_join!(output, drain)?;
            let status = child.wait().await?;
            report.failure.exit = Some(status.into());
            if !status.success() {
                report.failure.message =
                    "Cargo compilation failed; local routes were not replaced".into();
                return Ok(None);
            }
            if executable.is_none() {
                report.failure.message = "Cargo succeeded without a worker executable".into();
            }
            Ok::<_, YappinError>(executable)
        };
        match tokio::time::timeout(self.build_timeout, build).await {
            Ok(Ok(Some(executable))) => return Ok(executable),
            Ok(Ok(None)) => (),
            Ok(Err(cause)) => {
                report.failure.message = "Cargo output could not be processed".into();
                if let YappinError::HotReload { failure } = error(
                    Compilation,
                    "Cargo output could not be processed",
                    Some(&cause),
                    false,
                    false,
                    None,
                ) {
                    report.failure.source = failure.source;
                    report.failure.truncated |= failure.truncated;
                }
            }
            Err(_) => {
                report.failure.message = "Cargo compilation timed out".into();
                report.failure.timed_out = true;
            }
        }
        if report.failure.exit.is_none() {
            report.failure.exit = child.try_wait().ok().flatten().map(Into::into);
        }
        let _ = child.kill().await;
        Err(YappinError::HotReload {
            failure: Box::new(report.failure),
        })
    }
}
fn environment(command: &mut Process, build: bool) {
    command.env_clear();
    for key in ["PATH", "SystemRoot", "WINDIR", "TMPDIR", "TEMP", "TMP"] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    if build {
        for key in [
            "HOME",
            "USERPROFILE",
            "CARGO_HOME",
            "RUSTUP_HOME",
            "RUSTUP_TOOLCHAIN",
            "CC",
            "CXX",
            "AR",
        ] {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
    }
}

async fn spawn_worker(process: &mut Process) -> std::io::Result<Child> {
    // Concurrent fork/exec and executable copying can temporarily leave a
    // writable descriptor open on Linux. A failed spawn has executed no worker
    // code, so retry only that specific OS error, for at most 310 ms of waits.
    for attempt in 0..=5 {
        match process.spawn() {
            Err(error) if error.kind() == std::io::ErrorKind::ExecutableFileBusy && attempt < 5 => {
                tokio::time::sleep(Duration::from_millis(10 << attempt)).await;
            }
            result => return result,
        }
    }
    unreachable!("the final spawn attempt always returns")
}

#[cfg(test)]
mod spawn_tests {
    use super::*;

    #[tokio::test]
    async fn non_busy_spawn_errors_remain_errors() {
        let missing = std::env::temp_dir().join(format!("rappin-missing-{}", uuid::Uuid::new_v4()));
        let error = spawn_worker(&mut Process::new(missing)).await.unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn executable_busy_retry_recovers_after_writer_closes_and_is_bounded() {
        let directory =
            std::env::temp_dir().join(format!("rappin-spawn-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let _cleanup = DirectoryCleanup(Some(directory.clone()));
        let binary = directory.join("worker");
        std::fs::copy("/bin/true", &binary).unwrap();
        let writer = std::fs::OpenOptions::new()
            .write(true)
            .open(&binary)
            .unwrap();
        let release = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(40)).await;
            drop(writer);
        });
        let mut process = Process::new(&binary);
        process.kill_on_drop(true);
        let mut child = tokio::time::timeout(Duration::from_secs(2), spawn_worker(&mut process))
            .await
            .unwrap()
            .unwrap();
        assert!(child.wait().await.unwrap().success());
        release.await.unwrap();
        let _writer = std::fs::OpenOptions::new()
            .write(true)
            .open(&binary)
            .unwrap();
        let error = tokio::time::timeout(
            Duration::from_secs(2),
            spawn_worker(&mut Process::new(&binary)),
        )
        .await
        .unwrap()
        .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::ExecutableFileBusy);
    }
}
struct Io {
    child: Option<Child>,
    input: Option<ChildStdin>,
    output: BufReader<ChildStdout>,
    poisoned: bool,
}
// Also cleans up when a startup/watch future or the runtime is cancelled.
struct DirectoryCleanup(Option<PathBuf>);
impl Drop for DirectoryCleanup {
    fn drop(&mut self) {
        if let Some(path) = &self.0 {
            let _ = std::fs::remove_dir_all(path);
        }
    }
}
struct Generation {
    manifest: Manifest,
    io: Mutex<Io>,
    directory: PathBuf,
    pid: Option<u32>,
    timeout: Duration,
}
impl Generation {
    async fn start(
        executable: &Path,
        working_directory: Option<&Path>,
        startup: Duration,
        timeout: Duration,
    ) -> Result<Arc<Self>> {
        use super::diagnostics::error;
        use crate::ReloadPhase::{Protocol, Registration, Startup};
        let startup_error = |cause: std::io::Error| {
            error(
                Startup,
                "candidate executable could not be prepared",
                Some(&cause.into()),
                false,
                false,
                None,
            )
        };
        let source = tokio::fs::canonicalize(executable)
            .await
            .map_err(startup_error)?;
        let directory = std::env::temp_dir().join(format!("rappin-cog-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir(&directory)
            .await
            .map_err(startup_error)?;
        let mut cleanup = DirectoryCleanup(Some(directory.clone()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))
                .await
                .map_err(startup_error)?;
        }
        let setup = async {
            let target = directory.join(if cfg!(windows) {
                "worker.exe"
            } else {
                "worker"
            });
            tokio::fs::copy(&source, &target).await.map_err(startup_error)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                tokio::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o700)).await.map_err(startup_error)?;
            }
            let mut process = Process::new(target);
            environment(&mut process, false);
            process
                .current_dir(
                    working_directory.unwrap_or_else(|| source.parent().unwrap_or(Path::new("."))),
                )
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit())
                .kill_on_drop(true);
            let mut child = spawn_worker(&mut process).await.map_err(startup_error)?;
            let pid = child.id();
            let input = child.stdin.take().ok_or_else(|| error(Startup, "candidate stdin unavailable", None, true, false, None))?;
            let mut output =
                BufReader::new(child.stdout.take().ok_or_else(|| error(Startup, "candidate stdout unavailable", None, true, false, None))?);
            let ready = match tokio::time::timeout(startup, protocol::read(&mut output)).await {
                Ok(Ok(ready)) => ready,
                result => {
                    let (phase, message, cause, timed_out) = match result {
                        Err(_) => (Startup, "candidate startup timed out", None, true),
                        Ok(Err(cause)) => {
                            let exited = matches!(&cause, YappinError::Config(message) if message.contains("exited or closed"));
                            (if exited { Startup } else { Protocol }, "candidate did not send a valid ready frame", Some(cause), false)
                        },
                        _ => unreachable!(),
                    };
                    let mut failure = error(phase, message, cause.as_ref(), true, false, None).hot_reload_failure().expect("candidate report");
                    failure.timed_out = timed_out;
                    failure.exit = child.try_wait().ok().flatten().map(Into::into);
                    if failure.exit.is_none() && phase == Startup && !timed_out {
                        failure.exit = tokio::time::timeout(Duration::from_millis(25), child.wait()).await.ok().and_then(std::result::Result::ok).map(Into::into);
                    }
                    return Err(YappinError::HotReload { failure: Box::new(failure) });
                }
            };
            let manifest = match ready {
                Wire::Ready {
                    protocol: version,
                    manifest,
                } if (1..=protocol::VERSION).contains(&version)
                    && (version >= 5 || !manifest.listeners.contains(&Listener::ServerMessage)) => manifest,
                _ => return Err(error(Protocol, "candidate ready frame has an unsupported protocol", None, true, false, None)),
            };
            manifest.validate().map_err(|cause| error(Registration, "candidate manifest is invalid", Some(&cause), true, false, None))?;
            Ok(Arc::new(Self {
                manifest,
                io: Mutex::new(Io {
                    child: Some(child),
                    input: Some(input),
                    output,
                    poisoned: false,
                }),
                directory: directory.clone(),
                pid,
                timeout,
            }))
        }
        .await;
        if setup.is_ok() {
            cleanup.0 = None;
        }
        setup
    }
    async fn invoke(&self, event: CogEvent, client: &Client) -> Result<()> {
        self.invoke_message(event, client).await.map(|_| ())
    }
    async fn invoke_message(
        &self,
        event: CogEvent,
        client: &Client,
    ) -> Result<crate::MessageDisposition> {
        let expects_disposition = matches!(&event, CogEvent::Message { .. })
            && self.manifest.listeners.contains(&Listener::ServerMessage);
        let completed = std::sync::atomic::AtomicBool::new(false);
        let call = async {
            let mut io = self.io.lock().await;
            if io.poisoned {
                return Err(YappinError::Config(
                    "cog connection is no longer usable; reload it".into(),
                ));
            }
            // A cancelled/expired partial exchange must never be reused for another event.
            io.poisoned = true;
            protocol::write(
                io.input.as_mut().ok_or_else(protocol::protocol_error)?,
                &Wire::Invoke {
                    event: Box::new(event),
                },
            )
            .await?;
            let mut calls = 0;
            loop {
                match protocol::read(&mut io.output).await? {
                    Wire::Wait {
                        filter,
                        timeout_ms,
                        prompt,
                        channel,
                    } => {
                        calls += 1;
                        if calls > 128 {
                            return Err(YappinError::Config(
                                "worker exceeded 128 requests per event".into(),
                            ));
                        }
                        let result = async {
                            if timeout_ms == 0 || timeout_ms > 300_000 {
                                return Err(YappinError::validation(
                                    "timeout",
                                    "worker waits require 1 through 300000 milliseconds",
                                ));
                            }
                            let waiter = client
                                .events()
                                .collect::<crate::Event>()
                                .criteria(filter)
                                .timeout(Duration::from_millis(timeout_ms))
                                .limit(1)
                                .subscribe()?;
                            if let Some(message) = prompt {
                                let channel = channel.ok_or_else(|| {
                                    YappinError::validation("channel", "prompt requires a channel")
                                })?;
                                client.send_message(channel, &message).await?;
                            }
                            let event = waiter.wait().await?;
                            let value = client.worker_output(serde_json::to_value(event)?);
                            Ok::<crate::Event, YappinError>(serde_json::from_value(value)?)
                        }
                        .await;
                        let message = match result {
                            Ok(event) => Wire::EventResult {
                                event: Some(Box::new(event)),
                                error: None,
                                structured_error: None,
                            },
                            Err(error) => Wire::EventResult {
                                event: None,
                                error: Some(
                                    client
                                        .worker_output(Value::String(error.to_string()))
                                        .as_str()
                                        .unwrap_or("event wait failed")
                                        .into(),
                                ),
                                structured_error: Some(serde_json::from_value(
                                    client.worker_output(error.to_json()),
                                )?),
                            },
                        };
                        protocol::write(
                            io.input.as_mut().ok_or_else(protocol::protocol_error)?,
                            &message,
                        )
                        .await?;
                    }
                    Wire::Done {
                        disposition,
                        error,
                        structured_error,
                    } => {
                        if expects_disposition
                            && error.is_none()
                            && structured_error.is_none()
                            && disposition.is_none()
                        {
                            return Err(protocol::protocol_error());
                        }
                        io.poisoned = false;
                        completed.store(true, Ordering::SeqCst);
                        if let Some(error) = structured_error {
                            let error: crate::RemoteError = serde_json::from_value(
                                client.worker_output(serde_json::to_value(error)?),
                            )?;
                            return Err(error.into());
                        }
                        return match error {
                            Some(error) => Err(YappinError::Config(
                                client
                                    .worker_output(Value::String(error))
                                    .as_str()
                                    .unwrap_or("worker failed")
                                    .into(),
                            )),
                            None => Ok(disposition.unwrap_or(crate::MessageDisposition::Continue)),
                        };
                    }
                    request @ (Wire::Api { .. }
                    | Wire::ApiOnce { .. }
                    | Wire::Public { .. }
                    | Wire::Upload { .. }) => {
                        calls += 1;
                        if calls > 128 {
                            return Err(YappinError::Config(
                                "worker exceeded 128 API requests per event".into(),
                            ));
                        }
                        let response = match request {
                            Wire::Api { method, path, body } => {
                                match reqwest::Method::from_bytes(method.as_bytes()) {
                                    Ok(method) => {
                                        client
                                            .request_json::<Value, Value>(
                                                method,
                                                &path,
                                                body.as_ref(),
                                            )
                                            .await
                                    }
                                    Err(_) => Err(YappinError::validation(
                                        "method",
                                        "invalid worker API method",
                                    )),
                                }
                            }
                            Wire::ApiOnce { method, path, body } => {
                                match reqwest::Method::from_bytes(method.as_bytes()) {
                                    Ok(method) => {
                                        crate::Api::request_once(client, method, &path, body).await
                                    }
                                    Err(_) => Err(YappinError::validation(
                                        "method",
                                        "invalid worker API method",
                                    )),
                                }
                            }
                            Wire::Public { path } => client.public_json(&path).await,
                            Wire::Upload { path, body, files } => {
                                let files = protocol::read_upload(&mut io.output, files).await?;
                                client.request_upload(&path, body, &files).await
                            }
                            _ => unreachable!(),
                        };
                        let message = match response {
                            Ok(value) => Wire::ApiResult {
                                value: Some(client.worker_output(value)),
                                error: None,
                                structured_error: None,
                            },
                            Err(error) => Wire::ApiResult {
                                value: None,
                                structured_error: Some(serde_json::from_value(
                                    client.worker_output(error.to_json()),
                                )?),
                                error: Some(
                                    client
                                        .worker_output(Value::String(error.to_string()))
                                        .as_str()
                                        .unwrap_or("API request failed")
                                        .into(),
                                ),
                            },
                        };
                        protocol::write(
                            io.input.as_mut().ok_or_else(protocol::protocol_error)?,
                            &message,
                        )
                        .await?;
                    }
                    Wire::Permissions {
                        server,
                        actor,
                        prefer_cache,
                    } => {
                        calls += 1;
                        if calls > 128 {
                            return Err(YappinError::Config(
                                "worker exceeded 128 requests per event".into(),
                            ));
                        }
                        let result = crate::Api::permission_snapshot(
                            client,
                            server,
                            actor,
                            if prefer_cache {
                                crate::Freshness::PreferCache
                            } else {
                                crate::Freshness::Refresh
                            },
                        )
                        .await;
                        let message = match result {
                            Ok(snapshot) => Wire::ApiResult {
                                value: Some(client.worker_output(serde_json::to_value(snapshot)?)),
                                error: None,
                                structured_error: None,
                            },
                            Err(error) => Wire::ApiResult {
                                value: None,
                                error: Some(
                                    client
                                        .worker_output(Value::String(error.to_string()))
                                        .as_str()
                                        .unwrap_or("permission lookup failed")
                                        .into(),
                                ),
                                structured_error: Some(serde_json::from_value(
                                    client.worker_output(error.to_json()),
                                )?),
                            },
                        };
                        protocol::write(
                            io.input.as_mut().ok_or_else(protocol::protocol_error)?,
                            &message,
                        )
                        .await?;
                    }
                    _ => return Err(protocol::protocol_error()),
                }
            }
        };
        match tokio::time::timeout(self.timeout, call).await {
            Ok(Err(error)) if !completed.load(Ordering::SeqCst) => Err(YappinError::HandlerInterrupted {
                reason: "worker exchange ended before a completion frame; reconcile writes before reloading".into(),
                source: Some(Box::new(error)),
            }),
            Ok(result) => result,
            Err(_) => Err(YappinError::HandlerInterrupted {
                reason: "worker handler timed out; reconcile writes before reloading".into(), source: None,
            }),
        }
    }
}
impl Drop for Generation {
    fn drop(&mut self) {
        let io = self.io.get_mut();
        if let Some(mut child) = io.child.take() {
            let input = io.input.take();
            let directory = DirectoryCleanup(Some(self.directory.clone()));
            let poisoned = io.poisoned;
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                runtime.spawn(async move {
                    if !poisoned {
                        if let Some(mut input) = input {
                            let _ = protocol::write(&mut input, &Wire::Shutdown).await;
                        }
                        if tokio::time::timeout(Duration::from_secs(2), child.wait())
                            .await
                            .is_err()
                        {
                            let _ = child.kill().await;
                        }
                    } else {
                        let _ = child.kill().await;
                    }
                    let _ = child.wait().await;
                    if let Some(path) = &directory.0 {
                        let _ = tokio::fs::remove_dir_all(path).await;
                    }
                });
            } else {
                let _ = child.start_kill();
                drop(child);
                drop(directory);
            }
        }
    }
}
struct ProcessCog {
    generation: Arc<Generation>,
}
#[async_trait::async_trait]
impl Cog for ProcessCog {
    fn name(&self) -> &str {
        &self.generation.manifest.name
    }
    fn commands(&self) -> Vec<CogCommand> {
        self.worker_commands()
            .into_iter()
            .filter(|command| {
                !self
                    .generation
                    .manifest
                    .help
                    .iter()
                    .any(|info| info.name == command.name)
            })
            .collect()
    }
    fn registered_commands(&self) -> Vec<crate::commands::RegisteredCommand> {
        self.worker_commands()
            .into_iter()
            .filter_map(|command| {
                self.generation
                    .manifest
                    .help
                    .iter()
                    .find(|info| info.name == command.name)
                    .map(|info| crate::commands::RegisteredCommand {
                        info: info.clone(),
                        handler: command.handler,
                        preparation: None,
                    })
            })
            .collect()
    }
    fn buttons(&self) -> Vec<CogButton> {
        self.generation
            .manifest
            .buttons
            .iter()
            .map(|prefix| {
                let generation = self.generation.clone();
                let prefix = prefix.clone();
                let key = prefix.clone();
                CogButton::new(key, move |ctx| {
                    let generation = generation.clone();
                    let prefix = prefix.clone();
                    async move {
                        generation
                            .invoke(
                                CogEvent::Button {
                                    prefix,
                                    interaction: ctx.event,
                                },
                                &ctx.client,
                            )
                            .await
                    }
                })
            })
            .collect()
    }
    async fn on_reaction_add(&self, client: &Client, event: &crate::ReactionEvent) -> Result<()> {
        self.listen_result(
            Listener::ReactionAdd,
            CogEvent::ReactionAdd {
                event: event.clone(),
            },
            client,
        )
        .await
    }
    async fn on_reaction_remove(
        &self,
        client: &Client,
        event: &crate::ReactionEvent,
    ) -> Result<()> {
        self.listen_result(
            Listener::ReactionRemove,
            CogEvent::ReactionRemove {
                event: event.clone(),
            },
            client,
        )
        .await
    }
    async fn on_dm(&self, ctx: &MessageContext) -> Result<()> {
        self.listen_result(
            Listener::DmMessage,
            CogEvent::DmMessage {
                message: ctx.event.clone(),
            },
            &ctx.client,
        )
        .await
    }
    async fn on_message_result(&self, ctx: &MessageContext) -> Result<crate::MessageDisposition> {
        if self
            .generation
            .manifest
            .listeners
            .contains(&Listener::ServerMessage)
        {
            self.generation
                .invoke_message(
                    CogEvent::Message {
                        message: ctx.event.clone(),
                    },
                    &ctx.client,
                )
                .await
        } else {
            self.on_message(ctx).await;
            Ok(crate::MessageDisposition::Continue)
        }
    }
    async fn on_message(&self, ctx: &MessageContext) {
        self.listen(
            Listener::Message,
            CogEvent::Message {
                message: ctx.event.clone(),
            },
            &ctx.client,
        )
        .await;
    }
    async fn on_ui_interaction(&self, ctx: &crate::UiInteractionContext) -> Result<()> {
        if self
            .generation
            .manifest
            .listeners
            .contains(&Listener::UiInteraction)
        {
            self.generation
                .invoke(
                    CogEvent::UiInteraction {
                        event: ctx.data.clone(),
                    },
                    &ctx.context.client,
                )
                .await
                .map(|_| ())
        } else {
            Ok(())
        }
    }
    async fn on_interaction(&self, ctx: &InteractionContext) {
        self.listen(
            Listener::Interaction,
            CogEvent::Interaction {
                interaction: ctx.event.clone(),
            },
            &ctx.client,
        )
        .await;
    }
    async fn on_member_join(&self, client: &Client, event: &crate::MemberJoinEvent) {
        self.listen(
            Listener::MemberJoin,
            CogEvent::MemberJoin {
                event: event.clone(),
            },
            client,
        )
        .await;
    }
    async fn on_member_leave(&self, client: &Client, event: &crate::MemberLeaveEvent) {
        self.listen(
            Listener::MemberLeave,
            CogEvent::MemberLeave {
                event: event.clone(),
            },
            client,
        )
        .await;
    }
    async fn on_invite_create(&self, client: &Client, event: &crate::InviteEvent) {
        self.listen(
            Listener::InviteCreate,
            CogEvent::InviteCreate {
                event: event.clone(),
            },
            client,
        )
        .await;
    }
    async fn on_invite_delete(&self, client: &Client, event: &crate::InviteEvent) {
        self.listen(
            Listener::InviteDelete,
            CogEvent::InviteDelete {
                event: event.clone(),
            },
            client,
        )
        .await;
    }
}
impl ProcessCog {
    fn worker_commands(&self) -> Vec<CogCommand> {
        self.generation
            .manifest
            .commands
            .iter()
            .map(|command| {
                let generation = self.generation.clone();
                let name = command.name.clone();
                CogCommand::new(&command.name, &command.description, move |ctx| {
                    let generation = generation.clone();
                    let command = name.clone();
                    async move {
                        generation
                            .invoke(
                                CogEvent::Command {
                                    command,
                                    args: ctx.args,
                                    argv: ctx.argv,
                                    message: ctx.event,
                                },
                                &ctx.client,
                            )
                            .await
                    }
                })
            })
            .collect()
    }
    async fn listen_result(&self, kind: Listener, event: CogEvent, client: &Client) -> Result<()> {
        if self.generation.manifest.listeners.contains(&kind) {
            self.generation.invoke(event, client).await
        } else {
            Ok(())
        }
    }
    async fn listen(&self, kind: Listener, event: CogEvent, client: &Client) {
        if self.generation.manifest.listeners.contains(&kind) {
            if let Err(error) = self.generation.invoke(event, client).await {
                tracing::error!("Cog listener failed: {error}");
            }
        }
    }
}
#[derive(Clone)]
enum Source {
    Project(WorkerProject),
    Executable(PathBuf),
}
struct ReloadState {
    bot: crate::bot::WeakBotHandle,
    source: Source,
    name: String,
    current: RwLock<Option<Arc<Generation>>>,
    reload: Mutex<()>,
    number: AtomicU64,
}
/// Cloneable controller. A failed build/handshake/registration leaves the old generation installed.
#[derive(Clone)]
pub struct HotReload {
    inner: Arc<ReloadState>,
}
impl HotReload {
    pub async fn load(bot: BotHandle, project: WorkerProject) -> Result<Self> {
        Self::load_source(bot, Source::Project(project)).await
    }
    pub async fn from_executable(bot: BotHandle, path: impl Into<PathBuf>) -> Result<Self> {
        Self::load_source(bot, Source::Executable(path.into())).await
    }
    async fn prepare(source: &Source, client: &Client) -> Result<Arc<Generation>> {
        let result = async {
            match source {
                Source::Project(project) => {
                    let executable = project.build().await?;
                    let manifest =
                        tokio::fs::canonicalize(&project.manifest)
                            .await
                            .map_err(|cause| {
                                super::diagnostics::error(
                                    crate::ReloadPhase::Startup,
                                    "manifest disappeared after compilation",
                                    Some(&cause.into()),
                                    false,
                                    false,
                                    None,
                                )
                            })?;
                    Generation::start(
                        &executable,
                        manifest.parent(),
                        project.startup_timeout,
                        project.handler_timeout,
                    )
                    .await
                }
                Source::Executable(path) => {
                    Generation::start(
                        path,
                        None,
                        Duration::from_secs(10),
                        Duration::from_secs(120),
                    )
                    .await
                }
            }
        }
        .await;
        // Also scrub the parent's known credential, which need not be in env.
        result.map_err(|cause| match cause {
            YappinError::HotReload { failure } => {
                let failure =
                    serde_json::from_value(client.redact_credentials(
                        serde_json::to_value(failure).expect("candidate report"),
                    ))
                    .expect("redacted candidate report");
                YappinError::HotReload { failure }
            }
            cause => cause,
        })
    }
    async fn load_source(bot: BotHandle, source: Source) -> Result<Self> {
        let generation = Self::prepare(&source, bot.client()).await?;
        let name = generation.manifest.name.clone();
        if bot.has_cog(&name).await {
            return Err(super::diagnostics::error(
                crate::ReloadPhase::Registration,
                "a cog with this name is already installed",
                None,
                true,
                false,
                Some(bot.client()),
            ));
        }
        bot.install_worker(
            ProcessCog {
                generation: generation.clone(),
            },
            false,
        )
        .await?;
        Ok(Self {
            inner: Arc::new(ReloadState {
                bot: bot.downgrade(),
                source,
                name,
                current: RwLock::new(Some(generation)),
                reload: Mutex::new(()),
                number: AtomicU64::new(1),
            }),
        })
    }
    pub fn name(&self) -> &str {
        &self.inner.name
    }
    pub fn generation(&self) -> u64 {
        self.inner.number.load(Ordering::SeqCst)
    }
    pub async fn process_id(&self) -> Option<u32> {
        self.inner.current.read().await.as_ref().and_then(|g| g.pid)
    }
    pub async fn commands(&self) -> Vec<Command> {
        self.inner
            .current
            .read()
            .await
            .as_ref()
            .map(|g| g.manifest.commands.clone())
            .unwrap_or_default()
    }
    pub async fn reload(&self) -> Result<()> {
        let _reload = self.inner.reload.lock().await;
        if self.inner.current.read().await.is_none() {
            return Err(super::diagnostics::error(
                crate::ReloadPhase::Registration,
                "cog has been unloaded",
                None,
                false,
                false,
                None,
            ));
        }
        let bot = self.inner.bot.upgrade().ok_or_else(|| {
            super::diagnostics::error(
                crate::ReloadPhase::Registration,
                "parent bot has been dropped",
                None,
                false,
                false,
                None,
            )
        })?;
        let generation = Self::prepare(&self.inner.source, bot.client()).await?;
        if generation.manifest.name != self.inner.name {
            return Err(super::diagnostics::error(
                crate::ReloadPhase::Registration,
                "reload cannot change the cog name",
                None,
                true,
                false,
                Some(bot.client()),
            ));
        }
        bot.install_worker(
            ProcessCog {
                generation: generation.clone(),
            },
            true,
        )
        .await?;
        *self.inner.current.write().await = Some(generation);
        self.inner.number.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    /// Stop accepting new events; in-flight handlers retain their generation until done.
    pub async fn unload(&self) -> Result<()> {
        let _reload = self.inner.reload.lock().await;
        if let Some(bot) = self.inner.bot.upgrade() {
            bot.remove_cog(&self.inner.name).await?;
        }
        *self.inner.current.write().await = None;
        Ok(())
    }

    /// Watch Rust/TOML sources and Cargo.lock. Keep the returned guard alive.
    pub async fn watch(&self, interval: Duration) -> Result<WatchGuard> {
        if interval.is_zero() {
            return Err(YappinError::validation("interval", "must be positive"));
        }
        let source = self.inner.source.clone();
        let mut previous = fingerprint(source.clone()).await?;
        let controller = self.clone();
        let (status_tx, status) = tokio::sync::watch::channel(WatchStatus::default());
        let task = tokio::spawn(async move {
            loop {
                tokio::time::sleep(interval).await;
                let Some(bot) = controller.inner.bot.upgrade() else {
                    break;
                };
                if !bot.has_cog(controller.name()).await {
                    break;
                }
                drop(bot);
                let mut next_status = status_tx.borrow().clone();
                match fingerprint(source.clone()).await {
                    Ok(next) => {
                        next_status.scan_error = None;
                        if next != previous {
                            previous = next;
                            let error = controller.reload().await.err();
                            next_status.reload_failure =
                                error.as_ref().and_then(YappinError::hot_reload_failure);
                            next_status.reload_error = error.map(|e| e.to_string());
                            if let Some(error) = &next_status.reload_error {
                                tracing::warn!("Hot reload failed; active cog kept: {error}");
                            }
                        }
                    }
                    Err(error) => next_status.scan_error = Some(error.to_string()),
                }
                status_tx.send_if_modified(|status| {
                    if *status == next_status {
                        false
                    } else {
                        *status = next_status;
                        true
                    }
                });
            }
        });
        Ok(WatchGuard { task, status })
    }
}
/// Independent source scan and candidate reload failures.
/// A recovered scan clears only `scan_error`; a failed candidate remains visible
/// until a changed candidate is successfully installed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WatchStatus {
    pub scan_error: Option<String>,
    pub reload_error: Option<String>,
    pub reload_failure: Option<crate::HotReloadFailure>,
}
/// Dropping the guard stops watching; the currently installed cog keeps running.
pub struct WatchGuard {
    task: tokio::task::JoinHandle<()>,
    status: tokio::sync::watch::Receiver<WatchStatus>,
}
impl WatchGuard {
    pub fn status(&self) -> WatchStatus {
        self.status.borrow().clone()
    }
    /// Most recent scan error, or the last failed candidate if scanning is healthy.
    pub fn last_error(&self) -> Option<String> {
        let status = self.status.borrow();
        status
            .scan_error
            .clone()
            .or_else(|| status.reload_error.clone())
    }
}
impl Drop for WatchGuard {
    fn drop(&mut self) {
        self.task.abort();
    }
}
