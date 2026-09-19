# Benchmarking a PrismML-fork model (Bonsai 2 and friends)

Prism ML's ternary GGUFs (`PTQ1_0`, `PQ2_0`) cannot be loaded by stock
llama.cpp. Their model card is explicit about why, and it is not only the
quantisation types:

> Stock llama.cpp will not run these files. It rejects `PQ2_0` and `PTQ1_0` as
> unknown types, and it loads `Q2_0` without any warning and produces garbage,
> because it has no Hadamard activation runtime.

So measuring one means building EmailOps against
[PrismML-Eng/llama.cpp](https://github.com/PrismML-Eng/llama.cpp). This file is
the recipe. **Nothing here is shipped**: the fork is opt-in per command, the
default build keeps upstream llama.cpp, and no committed file changes.

## Why it is cheap

The fork's public C API is a strict superset of the upstream the bindings are
generated from: `include/llama.h` is identical function-for-function (155), and
`ggml/include/ggml.h` only *adds* four (`ggml_clamp_inplace`,
`ggml_gated_delta_net_rows`, `ggml_rope_set_offset`,
`ggml_gated_delta_net_set_raw_gates`). `llama-cpp-2`'s Rust wrapper needs no
changes at all. Only `llama-cpp-sys-2`'s own C++ shim does.

## Recipe

1. Clone the fork and copy the published sys crate next to it:

   ```bash
   WORK=/tmp/fork-port && mkdir -p "$WORK" && cd "$WORK"
   git clone --depth 1 https://github.com/PrismML-Eng/llama.cpp.git fork
   CRATE=~/.cargo/registry/src/*/llama-cpp-sys-2-0.1.156
   cp -R $CRATE llama-cpp-sys-2-prism && chmod -R u+w llama-cpp-sys-2-prism
   rm -rf llama-cpp-sys-2-prism/llama.cpp
   cp -R fork llama-cpp-sys-2-prism/llama.cpp
   rm -rf llama-cpp-sys-2-prism/llama.cpp/.git
   ```

2. Apply two patches to `llama-cpp-sys-2-prism/wrapper_common.cpp`, both caused
   by the fork's `common/` layer having moved on:

   - `common_fit_params` gained an `extra` model parameter before `log_level`.
     Pass `nullptr` — the fork documents that as "there is none".
   - `json_schema_to_grammar` now takes the fork's own `common_json` instead of
     `nlohmann::json`. Parse straight into it
     (`common_json::parse(schema_json)`) and include `common/json.h`.

3. Build and run with the patch supplied per invocation, so nothing committed
   changes and the normal build is untouched:

   ```bash
   LLAMA_PATCH_DIR="$WORK/llama-cpp-sys-2-prism" make bench-models \
     ARGS="--models qwen3.6-35b-a3b-ud-q4_k_xl,qwen35-bonsai-2-27b-pq2_0"
   ```

   `cargo` rewrites `src-tauri/Cargo.lock` while a path patch is active (it
   drops the registry source line). Restore it afterwards:
   `git checkout -- src-tauri/Cargo.lock`.

## The trap: name the GGUF so priming fires

`no_think_priming` (`ai/llama_cpp/runtime.rs`) decides whether to append the
closed `<think></think>` block from the **file name**
(`is_qwen3_model_path`: `name.starts_with("qwen3")`). Bonsai 2 is Qwen 3.5
family — its GGUF declares `general.architecture = qwen35` — but a file called
`bonsai-2-27b-pq2_0.gguf` does not match, gets no priming, spends the whole
generation reserve inside `<think>`, and `strip_reasoning` collapses every
reply to `""`. The eval then reports "empty reply" on every case and the model
looks broken.

Until the heuristic reads the architecture instead of the file name, name the
file so it matches — a hard link costs nothing:

```bash
cd "$EMAILOPS_DEMO_DIR/models/chat"
ln -f bonsai-2-27b-pq2_0.gguf qwen35-bonsai-2-27b-pq2_0.gguf
```

This is the same class of bug as the Gemma 4 chat-template fallback: an
integration detail that reads as "the model is bad".
