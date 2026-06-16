# Third-Party Software Licenses

This document tracks license terms and required attributions for third-party
software compiled into or distributed with EmailOps. Bundled model artifacts are
documented separately in [MODEL_LICENSES.md](MODEL_LICENSES.md).

## Embedded local inference runtime

The default ("llamacpp") build statically links the llama.cpp / ggml inference
runtime and its Rust bindings into the distributed binary.

### llama.cpp / ggml

- **Purpose:** embedded local LLM inference runtime (default local AI provider).
- **Source:** https://github.com/ggml-org/llama.cpp
- **Pulled in via:** the `llama-cpp-sys-2` crate, which vendors the llama.cpp
  sources (see `src-tauri/Cargo.toml`, `llamacpp` feature).
- **License:** MIT

```
MIT License

Copyright (c) 2023-2026 The ggml authors

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

### llama-cpp-rs (`llama-cpp-2`, `llama-cpp-sys-2`)

- **Purpose:** Rust bindings used to drive the llama.cpp runtime.
- **Source:** https://github.com/utilityai/llama-cpp-rs
- **License:** MIT OR Apache-2.0 (used here under the MIT option).

```
MIT License

Copyright (c) Dial AI

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## Notes

- Intel macOS release builds disable default features and do **not** embed the
  llama.cpp runtime (see the project `CLAUDE.md` macOS release section); on those
  builds the llama.cpp attribution above does not apply to the shipped binary.
- The full dependency tree carries many additional permissively licensed Rust
  crates. Run `cargo about` / `cargo deny` against `src-tauri/Cargo.toml` if a
  complete machine-generated attribution manifest is required for distribution.
