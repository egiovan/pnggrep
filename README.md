# pnggrep & smart_savefig

A lightweight, reproducible scientific visualization toolkit designed for high-performance computing (HPC) environments. Automatically embed complete Python source listings, execution context, and Git version metadata directly into generated figures (**PNG** and **SVG**), and search, inspect, or restore that code instantly using a standalone, zero-dependency Rust CLI.

---

## Why pnggrep?

In computational and experimental research, plots and figures are frequently separated from the scripts that produced them. Months or years later, reproducing an exact curve, finding which parameters were passed, or verifying git branch status can become difficult.

This repository pairs two complementary tools:

1. **`smart_savefig.py`**: A drop-in replacement for `matplotlib.pyplot.savefig`. It inspects the caller's call stack to embed the entire source script, parent directory, timestamp, and Git revision state into standard image metadata chunks.
2. **`pnggrep`**: A standalone, zero-dependency Rust command-line tool. It traverses large directory structures, bypassing heavy image decoders to search, preview, and extract embedded metadata across thousands of PNG and SVG files in milliseconds.

---

## Key Features

* **Zero External Rust Dependencies:** Compiles using only `std` and standard system `zlib` via FFI. Installs smoothly on offline HPC nodes, legacy clusters, and modern workstations alike (compatible with Rust 2018+).
* **Dual Format Support:**
* **PNG:** Parses uncompressed (`tEXt`), compressed deflate (`zTXt`), and international UTF-8 (`iTXt`) chunks directly at the byte level.
* **SVG:** Parses Dublin Core XML metadata (`<dc:description>`), automatically unpacking nested JSON dictionaries into structured two-level keys (e.g., `Description/source_code`).


* **Intelligent Directory Traversal:**
* **Default Blocklist:** Automatically skips common build and cache directories (`.git`, `__pycache__`, `target`, `build`, `dist`, `node_modules`, `.tox`, etc.).
* **Dynamic Virtual Environment Detection:** Automatically identifies and skips Python virtual environments regardless of folder name by detecting environment signatures (`pyvenv.cfg`, `conda-meta/`, or `bin/activate` + `bin/python`).
* **Custom Exclusions (`-s`, `--skip`, `--exclude`):** Exclude arbitrary directories or subpaths from traversal.
* **Clean Traversal Logging:** Informs the user on `stderr` whenever a directory is skipped, keeping `stdout` clean for UNIX piping and redirections.


* **Smart Git Status Tracking:** Tracks only actual uncommitted modifications to version-controlled files (`git status -uno`) and ensures the calling script itself is tracked (`git ls-files --error-unmatch`). Scratch files or temporary outputs left untracked in the working directory do not falsely flag the figure as dirty.
* **Security & DoS Hardening:**
* Protection against zlib decompression bombs (capped at 32 MB uncompressed).
* Strict memory allocation caps on PNG chunks (max 16 MB) and SVG files (max 32 MB).
* JSON recursion depth limits (max 8 levels) to prevent stack overflow attacks.
* Terminal ANSI escape sequence sanitization to prevent terminal spoofing or command injection when viewing untrusted files.


* **Direct Script Recovery (`--cat`):** Reconstruct runnable scripts directly from figure metadata using case- and separator-insensitive leaf matching (e.g., matching both `SourceCode` in PNG and `Description/source_code` in SVG).

---

## Installation

### 1. Build and Install `pnggrep` (Rust CLI)

```bash
git clone https://github.com/<your-username>/pnggrep.git
cd pnggrep
cargo build --release
mkdir -p ~/.local/bin
cp target/release/pnggrep ~/.local/bin/

```

Ensure `~/.local/bin` is in your shell `$PATH`:

```bash
export PATH="$HOME/.local/bin:$PATH"

```

### 2. Configure `smart_savefig.py` (Python)

Copy `smart_savefig.py` into your project directory or add the repository path to your `PYTHONPATH`:

```bash
export PYTHONPATH="/path/to/pnggrep:$PYTHONPATH"

```

---

## Quickstart Workflow

### 1. Save figures with embedded metadata

Replace `plt.savefig(...)` with `savefig(...)`:

```python
import matplotlib.pyplot as plt
import numpy as np
from smart_savefig import savefig

# Generate scientific plot
x = np.linspace(0, 10, 200)
y = np.sin(x) * np.exp(-x / 5.0)

fig, ax = plt.subplots()
ax.plot(x, y, label="Damped Sine")
ax.legend()

# Save as PNG or SVG - metadata is embedded automatically
savefig("damped_oscillation.png", dpi=300)
savefig("damped_oscillation.svg")

```

### 2. Search across image files

Search for functions, variables, or import statements across figures:

```bash
# Search current directory
pnggrep "np.sin" .

# Search a specific folder while excluding scratch runs
pnggrep "scipy.optimize" ~/simulations/figures/ -s old_archive --skip test_run

```

Output:

```text
[info] Skipping default ignored directory: ./.git
[info] Skipping virtual environment: ./analysis_py311
[info] Skipping excluded directory: ./simulations/figures/old_archive
simulations/figures/damped_oscillation.png [SourceCode]
     8: y = np.sin(x) * np.exp(-x / 5.0)

```

### 3. List metadata entries (`-l`)

Inspect embedded parameters and execution context with a preview:

```bash
# Default preview (up to 3 lines per entry)
pnggrep -l damped_oscillation.png

# Custom preview limit (e.g., up to 10 lines)
pnggrep -l -n 10 damped_oscillation.svg

# Full, untruncated listing
pnggrep -l -n 0 damped_oscillation.png

```

Output:

```text
damped_oscillation.png
  SourceFile: generate_plots.py
  SourceDirectory: /home/researcher/projects/mhd_analysis
  GitCommit: 8f4a2b1
  GitDirty: False
  GitScriptTracked: True
  Timestamp: 2026-09-13T10:15:30.123456
  SourceCode:
    | import matplotlib.pyplot as plt
    | import numpy as np
    | from smart_savefig import savefig
    ... [18 more lines omitted]

```

### 4. Restore the original script (`--cat`)

Extract the exact source code back into a Python file:

```bash
# Works identically on PNG and SVG files
pnggrep --cat SourceCode damped_oscillation.png > restored_script.py
pnggrep --cat SourceCode damped_oscillation.svg > restored_script.py

```

---

## CLI Reference

```text
pnggrep - Search and list embedded PNG and SVG metadata

USAGE:
    pnggrep <PATTERN> [PATH] [OPTIONS]  Search pattern in PNG/SVG metadata
    pnggrep -l [-n LINES] [PATH]        List all metadata keys and values
    pnggrep --cat <KEY> <FILE>          Print raw value of a metadata key

OPTIONS:
    -l                          List metadata mode (no pattern required)
    -n <LINES>                  Max lines to show per key in list mode [default: 3, 0=all]
    -s, --skip, --exclude <DIR> Skip specific directory during traversal (repeatable)
    -cat, --cat <KEY>           Dump exact value without formatting (case/separator-insensitive)
    -h, --help                  Show help information

```

---

## Project Structure

```text
pnggrep/
├── Cargo.toml          # Package manifest (optimized release profile, zero external crates)
├── Cargo.lock          # Locked dependency manifest
├── LICENSE             # MIT License
├── README.md           # Project documentation and specifications
├── smart_savefig.py    # Python helper module and metadata extractor
└── src/
    ├── main.rs         # CLI argument parsing and execution dispatch
    ├── model.rs        # MetadataEntry representations and key-resolution heuristics
    ├── png.rs          # Low-level PNG chunk decoder and zlib FFI bindings
    ├── svg.rs          # Standalone XML parser, unescaping, and JSON decoder
    └── walker.rs       # Directory traversal, venv detection, and terminal output

```

---

## Security Considerations

When inspecting image files originating from external or untrusted sources:

* **Memory Bounds:** Decompression buffers are strictly bounded to 32 MB to neutralise zip/zlib decompression bombs. PNG chunks exceeding 16 MB or SVG files exceeding 32 MB are skipped.
* **Terminal Safety:** All strings sent to standard output during search or list operations are stripped of ANSI escape sequences (`\x1b[...]`) and control characters, preventing terminal spoofing.
* **Parser Safety:** The custom SVG parser does not load external DTD entities, eliminating XML External Entity (XXE) and SSRF vulnerabilities.

---

## Acknowledgments & AI Disclosure

This project was conceived, directed, and verified by Edmondo Giovannozzi, with software architecture, performance optimization, and test suite scaffolding developed in collaboration with generative AI (**Gemini by Google**). All generated code and algorithms were manually reviewed, tested, and validated for correctness, security, and reproducibility.

---

## License

This project is licensed under the [MIT License](https://www.google.com/search?q=LICENSE).