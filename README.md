# pnggrep & smart_savefig

Reproducible scientific visualization toolkit: automatically embed complete Python source code and execution context into generated figures (**PNG** and **SVG**), and search or recover that code instantly with a blazing-fast, zero-dependency Rust CLI.

---

## Why pnggrep?

In computational science, plots are often shared or stored as standalone images (`.png` or `.svg`). Months later, identifying the exact script, commit hash, or parameter set that produced a specific figure can be difficult.

This repository provides two complementary tools:
1. **`smart_savefig.py`**: A drop-in replacement for `plt.savefig` that inspects the call stack and embeds the caller's entire Python source code, directory path, function name, timestamp, and Git commit hash directly into image metadata.
2. **`pnggrep`**: A standalone Rust CLI that searches, previews, and dumps code embedded inside thousands of PNG and SVG files in milliseconds—without opening external heavy image libraries or spawning Python runtimes.

---

## Features

- **Zero External Dependencies in Rust:** Links directly to standard system `zlib` via FFI. Compiles cleanly on older Linux distributions and offline HPC cluster nodes (compatible with Rust 2018+).
- **Fast Chunk-Level Inspection:** Bypasses pixel data (`IDAT`) using raw stream seeks. Never decodes images into memory.
- **Dual Format Support:**
  - **PNG:** Inspects uncompressed (`tEXt`), compressed zlib (`zTXt`), and international (`iTXt`) text chunks.
  - **SVG:** Parses Dublin Core XML metadata (`<dc:description>`), automatically unpacking nested JSON dictionaries.
- **Smart Directory Skipping:** Automatically skips `venv/`, `.git/`, `__pycache__/`, `target/`, and other non-relevant directories.
- **Direct Source Recovery (`--cat`):** Reconstruct original scripts directly from figure files.

---

## Installation

### 1. Build and Install `pnggrep` (Rust)

```bash
git clone [https://github.com/](https://github.com/)<your-username>/pnggrep.git
cd pnggrep
cargo build --release
mkdir -p ~/.local/bin
cp target/release/pnggrep ~/.local/bin/
```



Ensure `~/.local/bin` is present in your `$PATH`.

### 2. Configure `smart_savefig.py` (Python)

Copy `smart_savefig.py` into any directory included in your `PYTHONPATH`:

```bash
export PYTHONPATH="/path/to/pnggrep:$PYTHONPATH"

```

---

## Quickstart

### Step 1: Save figures with embedded metadata

```python
import matplotlib.pyplot as plt
import numpy as np
from smart_savefig import savefig

x = np.linspace(0, 10, 100)
plt.figure()
plt.plot(x, np.sin(x))

# Works for both PNG and SVG
savefig("sin_wave.png", dpi=300)
savefig("sin_wave.svg")

```

### Step 2: Search across all figures in a directory tree

```bash
# Search for function names, imports, or variable names
pnggrep "scipy.optimize" ~/simulations/figures/

# Search the current directory
pnggrep "def compute_flux"

```

### Step 3: Inspect metadata (`-l`)

```bash
# List metadata summary (default: up to 3 lines preview per field)
pnggrep -l

# Show up to 10 lines of preview
pnggrep -l -n 10

# Show full metadata without truncation
pnggrep -l -n 0

```

### Step 4: Extract and restore the original script (`--cat`)

Extract embedded source code back into a runnable Python file:

```bash
# Works seamlessly on both PNG and SVG:
pnggrep --cat SourceCode sin_wave.png > restored_script.py
pnggrep --cat SourceCode sin_wave.svg > restored_script.py

```

---

## CLI Usage Reference

```text
pnggrep - Search and list embedded PNG and SVG metadata

USAGE:
    pnggrep <PATTERN> [PATH]            Search pattern inside PNG/SVG metadata
    pnggrep -l [-n LINES] [PATH]        List all metadata keys and values
    pnggrep --cat <KEY> <FILE>          Print raw value of a metadata key

OPTIONS:
    -l                  List metadata mode (no pattern required)
    -n <LINES>          Max lines to show per key in list mode [default: 3, 0=all]
    -cat, --cat <KEY>   Dump exact value without formatting (case/separator-insensitive)
    -h, --help          Show help information

```

---

## License

This project is licensed under the MIT License. See the [LICENSE](https://www.google.com/search?q=LICENSE) file for details.

---

## Acknowledgments

This software was developed with the assistance of Google's Gemini AI for code implementation and test suite design, under human direction and verification.




