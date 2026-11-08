#!/usr/bin/env python3
"""Run README and guide doctests without configured bot credentials.

Requires cached all-feature dependencies, Python 3.11+, and Cargo. Dependencies
resolve offline; I/O snippets must use no_run. API signatures use text fences.
Compilation and doctests execute trusted code; this is not a sandbox.
"""
from pathlib import Path
import json
import os
import shutil
import subprocess
import tempfile
import tomllib


def main():
    root = Path(__file__).resolve().parents[1]
    package = tomllib.loads((root / "Cargo.toml").read_text())
    features = sorted(package["features"].keys() - {"default"})
    sources = [root / "README.md", *sorted((root / "docs").glob("*.md"))]
    with tempfile.TemporaryDirectory(prefix="rappin-guides-") as directory:
        work = Path(directory)
        (work / "src").mkdir()
        (work / "Cargo.toml").write_text(
            '[package]\nname="rappin-guide-check"\nversion="0.0.0"\n'
            'edition="2021"\npublish=false\n[features]\n'
            + "".join(f"{json.dumps(feature)}=[]\n" for feature in features)
            + '[dependencies]\nrappin={package="rappin-rs",path='
            + json.dumps(str(root), ensure_ascii=False) + ",features=" + json.dumps(features) + "}\n"
            + 'tokio={version="1",features=["full"]}\n'
            + 'uuid={version="1",features=["v4","serde"]}\n'
            + 'serde_json="1"\nasync-trait="0.1"\n'
            + 'reqwest={version="0.12",default-features=false}\n'
        )
        shutil.copyfile(root / "Cargo.lock", work / "Cargo.lock")
        (work / "src/lib.rs").write_text("\n".join(
            f"#[doc=include_str!({json.dumps(str(source), ensure_ascii=False)})]\npub mod document_{i} {{}}"
            for i, source in enumerate(sources)
        ))
        environment = os.environ.copy()
        for name in ["YAPPER_BOT_TOKEN", "YAPIN_WEBHOOK_SECRET", "WEBHOOK_SECRET"]:
            environment.pop(name, None)
        print(f"Checking README and {len(sources) - 1} guides offline", flush=True)
        subprocess.run([
            "cargo", "test", "--offline", "--manifest-path", str(work / "Cargo.toml"),
            "--all-features", "--doc", "--target-dir",
            os.environ.get("CARGO_TARGET_DIR", str(root / "target")),
        ], cwd=root, env=environment, check=True)


if __name__ == "__main__":
    main()
