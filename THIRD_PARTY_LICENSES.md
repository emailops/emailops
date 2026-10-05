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

## EO Docs: Word import

### mammoth

- **Purpose:** converts Word (.docx) files to HTML when they are imported into
  EO Docs.
- **Source:** https://github.com/mwilliamson/mammoth.js
- **Pulled in via:** the `mammoth` npm package, bundled into the webview.
- **License:** BSD-2-Clause (redistribution in binary form must reproduce the
  notice below)

```
Copyright (c) 2013, Michael Williamson
All rights reserved.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are met: 

1. Redistributions of source code must retain the above copyright notice, this
   list of conditions and the following disclaimer. 
2. Redistributions in binary form must reproduce the above copyright notice,
   this list of conditions and the following disclaimer in the documentation
   and/or other materials provided with the distribution. 

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND
ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED
WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT OWNER OR CONTRIBUTORS BE LIABLE FOR
ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES
(INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES;
LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND
ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
(INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
```

## EO Docs: PDF export

### pdfmake

- **Purpose:** lays out and writes the PDF when an EO Docs document is exported.
- **Source:** https://github.com/bpampuch/pdfmake
- **Pulled in via:** the `pdfmake` npm package, bundled into the webview and loaded on
  first export.
- **License:** MIT

```
The MIT License (MIT)

Copyright (c) 2014-2015 bpampuch
              2016-2026 liborm85

Permission is hereby granted, free of charge, to any person obtaining a copy of
this software and associated documentation files (the "Software"), to deal in
the Software without restriction, including without limitation the rights to
use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of
the Software, and to permit persons to whom the Software is furnished to do so,
subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS
FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR
COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER
IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN
CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
```

### Roboto (font)

- **Purpose:** the typeface of exported PDFs, embedded through pdfmake's default font
  bundle (`pdfmake/build/vfs_fonts`).
- **Source:** https://github.com/googlefonts/roboto
- **License:** Apache License 2.0 (Google relicensed later Roboto releases under the SIL
  Open Font License 1.1; both allow bundling and embedding the font).

## Notes

- Intel macOS release builds disable default features and do **not** embed the
  llama.cpp runtime (see the project `CLAUDE.md` macOS release section); on those
  builds the llama.cpp attribution above does not apply to the shipped binary.
- The full dependency tree carries many additional permissively licensed Rust
  crates. Run `cargo about` / `cargo deny` against `src-tauri/Cargo.toml` if a
  complete machine-generated attribution manifest is required for distribution.
