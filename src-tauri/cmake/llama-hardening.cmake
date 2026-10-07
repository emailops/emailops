# Binary-hardening flags for the llama.cpp / ggml C and C++ code.
#
# llama-cpp-sys-2's build script forwards every CMAKE_* environment variable to
# CMake as a cache entry, and `src-tauri/.cargo/config.toml` sets
# CMAKE_PROJECT_INCLUDE to this file, so CMake includes it at the end of
# llama.cpp's top-level project() call. include_guard(GLOBAL) skips the second
# inclusion from ggml's own project(); ggml is added as a subdirectory after
# this point and inherits everything set here.
#
# Only flags the toolchain does not already apply are added (see each branch).
#
# A warm build cache hides edits to this file: cmake-rs skips the configure
# step whenever the llama-cpp-sys-2 OUT_DIR already holds a CMakeCache.txt. The
# release workflow's rust-cache key hashes src-tauri/.cargo/config.toml (not
# this file), so touch the comment there when changing a flag here, and check
# the release artifacts afterwards.

include_guard(GLOBAL)

if(MSVC)
  # DASA 2.1.2 (Windows Control Flow Guard). /guard:cf at compile time emits
  # the CFG metadata; at link time it sets IMAGE_DLLCHARACTERISTICS_GUARD_CF on
  # the DLLs CMake links itself (ggml*.dll, llama*.dll and the dynamic backend
  # modules). The static libraries linked into emailops.exe get their final
  # /guard:cf from rustc's `-C control-flow-guard` (see .cargo/config.toml).
  # Compile flag limited to C/C++: nvcc rejects /guard:cf, so ggml-cuda's .cu
  # host code stays uninstrumented while the DLL is still CFG-linked. The link
  # flag goes through CMAKE_*_LINKER_FLAGS rather than add_link_options so it
  # never reaches nvcc's device-link step.
  # /DYNAMICBASE, /NXCOMPAT and /HIGHENTROPYVA are link.exe's defaults for x64
  # images; v0.6.14's PE files already carry all three.
  add_compile_options("$<$<COMPILE_LANGUAGE:C,CXX>:/guard:cf>")
  string(APPEND CMAKE_SHARED_LINKER_FLAGS " /guard:cf")
  string(APPEND CMAKE_MODULE_LINKER_FLAGS " /guard:cf")
endif()
