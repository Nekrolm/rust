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
profile_crates = os.environ.get("RELOCATION_TIME_PASSES_CRATES", "").split(",")
if "--crate-name" in command and command[command.index("--crate-name") + 1] in profile_crates:
    command.append("-Ztime-passes")
chained = os.environ.get("RELOCATION_CHAINED_RUSTC_WRAPPER")
if chained:
    command = [chained, *command]
result = subprocess.run(command, stderr=subprocess.PIPE)
stem.with_suffix(".stderr").write_bytes(result.stderr)
stem.with_suffix(".json").write_text(json.dumps({"command": command, "exit_code": result.returncode}) + "\n")
sys.stderr.buffer.write(result.stderr)
sys.exit(result.returncode)
