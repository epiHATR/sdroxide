//! Builds the vendored RADE C library and the FARGAN/LPCNet shim, then
//! generates raw bindings for both.
//!
//! `vendor/rade_c` is a git submodule, used unmodified. Its CMake downloads and
//! autotools-builds the patched Opus that carries the vocoder
//! (`--enable-osce --enable-dred`, plus rade_c's two `opus-nnet.*.diff`
//! patches), so the first build needs network access and
//! autoconf/automake/libtool.
//!
//! Opus's `autogen.sh` fetches ~175 MB of DNN weights from
//! `media.xiph.org`. That archive is kept outside `target/` (see
//! [`opus_model_cache_dir`]) and copied into each Opus source tree before
//! configure, so `cargo clean` does not force another download.
//!
//! Upstream builds `librade` as a shared object. We want it static — a shared
//! library would have to be shipped and found next to the executable on three
//! platforms, each with its own rules. So a small generated wrapper project
//! adds a second target over *the same source list*, read back off upstream's
//! own target rather than copied, so it cannot drift when the submodule moves.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};

/// SHA-256 of the DNN weight tarball pinned by Opus `autogen.sh` at the
/// commit `BuildOpus.cmake` fetches (`940d4e5a…`). Bump both together when
/// the submodule moves Opus.
const OPUS_MODEL_HASH: &str = "4ed9445b96698bad25d852e912b41495ddfa30c8dbc8a55f9cde5826ed793453";

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());

    let rade_c = manifest.join("../../vendor/rade_c");
    if !rade_c.join("src/rade_api.h").exists() {
        panic!(
            "vendored RADE sources are missing at {}\n\
             run: git submodule update --init --recursive",
            rade_c.display()
        );
    }
    let rade_c = rade_c.canonicalize().expect("canonicalize vendor/rade_c");

    let opus_model = ensure_opus_model(&manifest);
    println!("cargo:rerun-if-env-changed=SDROXIDE_OPUS_MODEL_CACHE");
    println!("cargo:rerun-if-env-changed=XDG_CACHE_HOME");

    let wrapper = write_wrapper_project(&out, &rade_c);

    // Always Release: the neural encoder/decoder is unusably slow unoptimized,
    // including under a plain `cargo build`.
    let dst = cmake::Config::new(&wrapper)
        .define("RADE_C_DIR", rade_c.to_string_lossy().as_ref())
        .define("OPUS_MODEL_TAR", opus_model.to_string_lossy().as_ref())
        .profile("Release")
        .build_target("rade_static")
        .build();

    let build = dst.join("build");
    // ExternalProject's default layout for rade_c's `build_opus` target, which
    // is configured BUILD_IN_SOURCE so its source and binary dirs coincide.
    let opus_src = build.join("build_opus-prefix/src/build_opus");
    let opus_lib_dir = opus_src.join(".libs");
    assert!(
        opus_lib_dir.is_dir(),
        "expected the Opus build at {} — did rade_c's ExternalProject change layout?",
        opus_lib_dir.display()
    );

    let includes: Vec<PathBuf> = vec![
        rade_c.join("src"),
        opus_src.clone(),
        opus_src.join("dnn"),
        opus_src.join("celt"),
        opus_src.join("include"),
    ];

    // The vocoder half: RADE's own API stops at feature vectors, so speech <->
    // features goes through a small C shim over Opus's FARGAN/LPCNet.
    let mut shim = cc::Build::new();
    shim.file(manifest.join("src/shim.c")).define("HAVE_CONFIG_H", None).opt_level(2);
    for inc in &includes {
        shim.include(inc);
    }
    shim.compile("sdroxide_rade_shim");

    println!("cargo:rustc-link-search=native={}", build.display());
    println!("cargo:rustc-link-lib=static=rade_static");
    println!("cargo:rustc-link-search=native={}", opus_lib_dir.display());
    println!("cargo:rustc-link-lib=static=opus");
    println!("cargo:rustc-link-lib=dylib=m");

    let mut builder = bindgen::Builder::default()
        .header(rade_c.join("src/rade_api.h").to_string_lossy())
        .header(manifest.join("src/shim.h").to_string_lossy())
        .allowlist_function("rade_.*")
        .allowlist_function("sdrx_voc_.*")
        .allowlist_type("RADE_COMP")
        .allowlist_var("RADE_.*")
        // `struct rade` is fully defined in the header — it embeds the V1 and
        // V2 transmit and receive state inline — but we only ever hold a
        // pointer to one. Blocklisting it keeps Rust off that layout entirely;
        // see the opaque declaration in sys.rs.
        .blocklist_type("rade")
        .layout_tests(false)
        .derive_debug(false)
        .clang_arg("-DHAVE_CONFIG_H");
    for inc in &includes {
        builder = builder.clang_arg(format!("-I{}", inc.display()));
    }
    builder
        .generate()
        .expect("bindgen rade_api.h + shim.h")
        .write_to_file(out.join("bindings.rs"))
        .expect("write bindings.rs");

    println!("cargo:rerun-if-changed=src/shim.c");
    println!("cargo:rerun-if-changed=src/shim.h");
    println!("cargo:rerun-if-changed={}", rade_c.join("src").display());
    println!("cargo:rerun-if-changed={}", rade_c.join("cmake").display());
}

/// Durable directory for the Opus DNN weight tarball. Survives `cargo clean`.
///
/// Order: `SDROXIDE_OPUS_MODEL_CACHE`, `$XDG_CACHE_HOME/sdroxide/opus_models`,
/// `~/.cache/sdroxide/opus_models`, `%LOCALAPPDATA%\sdroxide\opus_models`,
/// then the repo's gitignored `.cache/opus_models`.
fn opus_model_cache_dir(manifest: &Path) -> PathBuf {
    if let Ok(p) = std::env::var("SDROXIDE_OPUS_MODEL_CACHE") {
        return PathBuf::from(p);
    }
    if let Ok(xdg) = std::env::var("XDG_CACHE_HOME") {
        return PathBuf::from(xdg).join("sdroxide").join("opus_models");
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".cache/sdroxide/opus_models");
    }
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        return PathBuf::from(local).join("sdroxide").join("opus_models");
    }
    manifest.join("../../.cache/opus_models")
}

fn opus_model_name() -> String {
    format!("opus_data-{OPUS_MODEL_HASH}.tar.gz")
}

/// Ensure the weight tarball is on disk under the durable cache and return its path.
fn ensure_opus_model(manifest: &Path) -> PathBuf {
    let cache_dir = opus_model_cache_dir(manifest);
    std::fs::create_dir_all(&cache_dir).unwrap_or_else(|e| {
        panic!("create Opus model cache {}: {e}", cache_dir.display());
    });
    let dest = cache_dir.join(opus_model_name());
    if dest.is_file() && sha256_hex(&dest).ok().as_deref() == Some(OPUS_MODEL_HASH) {
        return dest;
    }
    if dest.is_file() {
        let _ = std::fs::remove_file(&dest);
    }

    // Promote a good copy left under target/ from an earlier build.
    if let Some(found) = find_cached_opus_model(manifest) {
        std::fs::copy(&found, &dest).unwrap_or_else(|e| {
            panic!("copy {} -> {}: {e}", found.display(), dest.display());
        });
        println!(
            "cargo:warning=promoted Opus DNN weights from {} to {}",
            found.display(),
            dest.display()
        );
        return dest;
    }

    let url = format!("https://media.xiph.org/opus/models/{}", opus_model_name());
    println!("cargo:warning=downloading Opus DNN weights to {} (~175 MB, once)", dest.display());
    download(&url, &dest);
    let h = sha256_hex(&dest).unwrap_or_else(|e| {
        panic!("could not hash {}: {e}", dest.display());
    });
    if h == OPUS_MODEL_HASH {
        dest
    } else {
        panic!(
            "Opus model checksum mismatch for {}\n  expected {OPUS_MODEL_HASH}\n  got      {h}\n\
             delete that file and rebuild",
            dest.display()
        );
    }
}

fn find_cached_opus_model(manifest: &Path) -> Option<PathBuf> {
    let name = opus_model_name();
    let workspace = manifest.join("../..");
    for profile in ["debug", "release"] {
        let builds = workspace.join(format!("target/{profile}/build"));
        let Ok(entries) = std::fs::read_dir(&builds) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry
                .path()
                .join("out/build/build_opus-prefix/src/build_opus")
                .join(&name);
            if path.is_file() && sha256_hex(&path).ok().as_deref() == Some(OPUS_MODEL_HASH) {
                return Some(path);
            }
        }
    }
    // Repo-local fallback (gitignored).
    let local = workspace.join(".cache/opus_models").join(&name);
    if local.is_file() && sha256_hex(&local).ok().as_deref() == Some(OPUS_MODEL_HASH) {
        return Some(local);
    }
    None
}

fn sha256_hex(path: &Path) -> Result<String, String> {
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    // sha2 0.11's Output no longer implements LowerHex; format bytes like digi.
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

fn download(url: &str, dest: &Path) {
    let partial = dest.with_extension("tar.gz.partial");
    let _ = std::fs::remove_file(&partial);
    let status = if Command::new("curl").arg("--version").output().is_ok() {
        Command::new("curl")
            .args(["-fL", "--retry", "5", "--retry-delay", "2", "-o"])
            .arg(&partial)
            .arg(url)
            .status()
    } else {
        Command::new("wget")
            .args(["-O"])
            .arg(&partial)
            .arg(url)
            .status()
    };
    match status {
        Ok(s) if s.success() => {
            std::fs::rename(&partial, dest).unwrap_or_else(|e| {
                panic!("rename {} -> {}: {e}", partial.display(), dest.display());
            });
        }
        Ok(s) => panic!("download of {url} failed with {s}"),
        Err(e) => panic!("download of {url} failed (need curl or wget): {e}"),
    }
}

/// Generate the wrapper CMake project into `OUT_DIR` and return its path.
///
/// It mirrors rade_c's own top-level `CMakeLists.txt` (include `BuildOpus`,
/// then add `src`) and adds one static target built from upstream's own source
/// list. `BuildOpus.cmake` resolves its Opus patches relative to
/// `CMAKE_SOURCE_DIR`, which is now *this* project, so the two `.diff` files
/// are copied in alongside. A patched copy of `BuildOpus.cmake` seeds the
/// cached DNN tarball into the Opus source tree before `autogen.sh` runs.
/// On Windows the C sources are copied into the build tree as well — see the
/// comment in the generated file.
fn write_wrapper_project(out: &Path, rade_c: &Path) -> PathBuf {
    let wrapper = out.join("wrapper");
    std::fs::create_dir_all(wrapper.join("cmake")).expect("create wrapper cmake dir");
    std::fs::create_dir_all(wrapper.join("src")).expect("create wrapper src dir");
    for diff in ["opus-nnet.h.diff", "opus-nnet.c.diff"] {
        std::fs::copy(rade_c.join("src").join(diff), wrapper.join("src").join(diff))
            .unwrap_or_else(|e| panic!("copy {diff}: {e}"));
    }

    let upstream = std::fs::read_to_string(rade_c.join("cmake/BuildOpus.cmake"))
        .expect("read vendor/rade_c/cmake/BuildOpus.cmake");
    let patched = patch_build_opus_cmake(&upstream);
    if patched == upstream {
        panic!(
            "failed to patch BuildOpus.cmake PATCH_COMMAND to seed Opus DNN weights — \
             did upstream change the patch line?"
        );
    }
    std::fs::write(wrapper.join("cmake/BuildOpus.cmake"), patched)
        .expect("write patched BuildOpus.cmake");

    std::fs::write(
        wrapper.join("CMakeLists.txt"),
        r#"# Generated by sdroxide-rade/build.rs — do not edit.
cmake_minimum_required(VERSION 3.16)
project(sdroxide_rade C)
set(CMAKE_C_STANDARD 11)
set(CMAKE_POSITION_INDEPENDENT_CODE ON)

if(NOT DEFINED OPUS_MODEL_TAR)
    message(FATAL_ERROR "OPUS_MODEL_TAR must be set (path to opus_data-*.tar.gz cache)")
endif()

# Fetches and patches the FARGAN/LPCNet-enabled Opus, and defines the imported
# `opus` target plus its include directories for everything below.
# Local copy of rade_c's BuildOpus.cmake with PATCH_COMMAND extended to seed
# OPUS_MODEL_TAR so media.xiph.org is not hit on every clean build.
include(${CMAKE_CURRENT_SOURCE_DIR}/cmake/BuildOpus.cmake)

# Upstream's own targets, unmodified. We never build its shared `rade`; we only
# borrow the target's source list.
add_subdirectory(${RADE_C_DIR}/src rade_c_src)

get_target_property(RADE_SOURCES rade SOURCES)
get_target_property(RADE_SRC_DIR rade SOURCE_DIR)

if(WIN32)
    # The windows-gnu build runs through MSYS make, whose makefile parser has
    # no notion of drive letters. For a source outside the build tree CMake
    # emits the rule line
    #     CMakeFiles/rade_static.dir/....obj: D:/.../rade_api.c
    # whose second colon makes it a malformed static pattern rule — make stops
    # with "target pattern contains no '%'". Compiling from copies inside the
    # binary directory keeps every path in the generated makefiles relative.
    set(RADE_COPY_DIR ${CMAKE_CURRENT_BINARY_DIR}/rade_src)
    set(RADE_ABS_SOURCES ${RADE_SOURCES})
    list(TRANSFORM RADE_ABS_SOURCES PREPEND "${RADE_SRC_DIR}/")
    file(GLOB RADE_HEADERS "${RADE_SRC_DIR}/*.h")
    file(COPY ${RADE_ABS_SOURCES} ${RADE_HEADERS} DESTINATION ${RADE_COPY_DIR})
    list(TRANSFORM RADE_SOURCES PREPEND "${RADE_COPY_DIR}/")
else()
    list(TRANSFORM RADE_SOURCES PREPEND "${RADE_SRC_DIR}/")
endif()

add_library(rade_static STATIC ${RADE_SOURCES})
target_include_directories(rade_static PRIVATE ${RADE_SRC_DIR})
target_compile_definitions(rade_static PRIVATE IS_BUILDING_RADE_API=1)
target_link_libraries(rade_static opus m)
"#,
    )
    .expect("write wrapper CMakeLists.txt");
    wrapper
}

/// Extend every `PATCH_COMMAND` so the cached DNN tarball is present before
/// Opus `autogen.sh` runs `dnn/download_model.sh`.
fn patch_build_opus_cmake(upstream: &str) -> String {
    let old = r#"PATCH_COMMAND sh -c "patch dnn/nnet.h < ${CMAKE_SOURCE_DIR}/src/opus-nnet.h.diff && patch dnn/nnet.c < ${CMAKE_SOURCE_DIR}/src/opus-nnet.c.diff""#;
    let new = format!(
        "PATCH_COMMAND ${{CMAKE_COMMAND}} -E copy \"${{OPUS_MODEL_TAR}}\" \"opus_data-{OPUS_MODEL_HASH}.tar.gz\"\n\
    COMMAND sh -c \"patch dnn/nnet.h < ${{CMAKE_SOURCE_DIR}}/src/opus-nnet.h.diff && patch dnn/nnet.c < ${{CMAKE_SOURCE_DIR}}/src/opus-nnet.c.diff\""
    );
    upstream.replace(old, &new)
}
