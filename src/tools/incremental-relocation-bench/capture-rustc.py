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
mode = os.environ.get("RELOCATION_EXPERIMENT_MODE")
if mode not in (None, "off", "on"):
    raise RuntimeError("unknown controlled experiment mode")
if mode == "on":
    command.append("-Zincremental-source-relocation")
if os.environ.get("RELOCATION_INJECT_REPORT") == "1":
    command.append("-Zincremental-relocation-report")
profile_crates = os.environ.get("RELOCATION_TIME_PASSES_CRATES", "").split(",")
if "--crate-name" in command and command[command.index("--crate-name") + 1] in profile_crates:
    command.append("-Ztime-passes")
process = subprocess.Popen(command, stderr=subprocess.PIPE)
with stem.with_suffix(".stderr").open("wb") as captured:
    while block := os.read(process.stderr.fileno(), 65536):
        captured.write(block)
        sys.stderr.buffer.write(block)
        sys.stderr.buffer.flush()
process.stderr.close()
status = process.wait()
stem.with_suffix(".json").write_text(json.dumps({"command": command, "exit_code": status, "controlled_experiment": mode}) + "\n")
sys.exit(status)
