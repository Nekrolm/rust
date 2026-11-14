#!/usr/bin/env python3
import json
import os
from pathlib import Path
import subprocess
import sys
import uuid


directory = Path(os.environ["RELOCATION_RUSTC_CAPTURE_DIR"])
directory.mkdir(parents=True, exist_ok=True)
stem = directory / str(uuid.uuid4())
command = sys.argv[1:]
if os.environ.get("RELOCATION_INJECT_REPORT") == "1":
    command.append("-Zincremental-relocation-report")
profile_crates = os.environ.get("RELOCATION_TIME_PASSES_CRATES", "").split(",")
if "--crate-name" in command and command[command.index("--crate-name") + 1] in profile_crates:
    command.append("-Ztime-passes")
result = subprocess.run(command, stderr=subprocess.PIPE)
stem.with_suffix(".stderr").write_bytes(result.stderr)
stem.with_suffix(".json").write_text(json.dumps({"command": command, "exit_code": result.returncode}) + "\n")
sys.stderr.buffer.write(result.stderr)
sys.exit(result.returncode)
