use std::path::PathBuf;
use std::process::Command;

fn main() {
    let target = std::env::var("TARGET").unwrap_or_default();
    let host = std::env::var("HOST").unwrap_or_default();
    let is_cross = !target.is_empty() && target != host;

    // Always compile ncnn from source as a static library (cached, only once)
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR not set"));
    let ncnn_root = out_dir.join("ncnn");
    let install_dir = ncnn_root.join("install");

    if !install_dir.join("lib").join("libncnn.a").exists() {
        download_and_build_ncnn(&ncnn_root, &install_dir, &target);
    }

    println!(
        "cargo:rustc-link-search=native={}",
        install_dir.join("lib").display()
    );
    println!("cargo:rustc-link-lib=static=ncnn");

    // All platforms need the C++ runtime because ncnn is a C++ static lib.
    // mupdf-sys uses -nodefaultlibs, so we must link c++ explicitly.
    if target.contains("linux") {
        println!("cargo:rustc-link-search=native=/opt/homebrew/opt/zlib/lib");
        println!("cargo:rustc-link-arg=-lc++");
        println!("cargo:rustc-link-arg=-lc++abi");
        println!("cargo:rustc-link-arg=-lm");
        println!("cargo:rustc-link-arg=-ldl");
        println!("cargo:rustc-link-arg=-lpthread");
        println!("cargo:rustc-link-arg=-lrt");
        println!("cargo:rustc-link-arg=-lutil");
        println!("cargo:rustc-link-lib=static=z");
    } else if target.contains("darwin") || target.contains("apple") {
        // macOS: mupdf-sys uses -nodefaultlibs, need to add C++ runtime explicitly
        println!("cargo:rustc-link-arg=-lc++");
        println!("cargo:rustc-link-arg=-lc++abi");
    } else if target.contains("windows") {
        println!("cargo:rustc-link-arg=-lstdc++");
        println!("cargo:rustc-link-arg=-lwinpthread");
    }
}

fn download_and_build_ncnn(ncnn_root: &PathBuf, install_dir: &PathBuf, target: &str) {
    let src_dir = ncnn_root.join("src");
    let build_dir = ncnn_root.join("build");

    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::create_dir_all(&build_dir).unwrap();
    std::fs::create_dir_all(&install_dir).unwrap();

    // Download ncnn source if not present
    if !src_dir.join("CMakeLists.txt").exists() {
        let status = Command::new("git")
            .args([
                "clone",
                "--depth=1",
                "--branch=20240820",
                "https://github.com/Tencent/ncnn.git",
                src_dir.to_str().unwrap(),
            ])
            .status()
            .expect("failed to clone ncnn");
        assert!(status.success(), "ncnn clone failed");
    }

    let prefix_arg = format!("-DCMAKE_INSTALL_PREFIX={}", install_dir.display());

    // Cross-compilation: tell cmake to use an explicit Linux toolchain.
    // Prefer env overrides, otherwise fall back to zig wrappers that work on macOS.
    let mut cross_flags: Vec<String> = Vec::new();
    let target_var = target.replace('-', "_");
    let cc = std::env::var(format!("CC_{}", target_var)).or_else(|_| std::env::var("CC"));
    let cxx = std::env::var(format!("CXX_{}", target_var)).or_else(|_| std::env::var("CXX"));
    if target.contains("musl") || target.contains("linux") {
        let zig_wrapper_dir = "/tmp/zig-musl";
        let cc = cc.unwrap_or_else(|_| format!("{zig_wrapper_dir}/x86_64-linux-gnu-gcc"));
        let cxx = cxx.unwrap_or_else(|_| format!("{zig_wrapper_dir}/x86_64-linux-gnu-g++"));
        // Use explicit toolchain pieces so CMake does not invent host tools.
        cross_flags.push(format!("-DCMAKE_C_COMPILER={}", cc));
        cross_flags.push(format!("-DCMAKE_CXX_COMPILER={}", cxx));
        cross_flags.push("-DCMAKE_LINKER=/opt/homebrew/bin/x86_64-linux-gnu-ld".to_string());
        cross_flags.push("-DCMAKE_AR=/opt/homebrew/bin/x86_64-linux-gnu-ar".to_string());
        cross_flags.push("-DCMAKE_RANLIB=/opt/homebrew/bin/x86_64-linux-gnu-ranlib".to_string());
        cross_flags.push("-DCMAKE_TRY_COMPILE_TARGET_TYPE=STATIC_LIBRARY".to_string());
    } else if target.contains("windows") {
        if let Ok(ref cc) = cc {
            cross_flags.push(format!("-DCMAKE_C_COMPILER={}", cc));
        }
        if let Ok(ref cxx) = cxx {
            cross_flags.push(format!("-DCMAKE_CXX_COMPILER={}", cxx));
        }
        cross_flags.push("-DCMAKE_SYSTEM_NAME=Windows".to_string());
        cross_flags.push(format!(
            "-DCMAKE_SYSTEM_PROCESSOR={}",
            if target.starts_with("x86_64") { "x86_64" } else { "x86" }
        ));
        cross_flags.push("-DCMAKE_TRY_COMPILE_TARGET_TYPE=STATIC_LIBRARY".to_string());
    } else {
        if let Ok(ref cc) = cc {
            cross_flags.push(format!("-DCMAKE_C_COMPILER={}", cc));
        }
        if let Ok(ref cxx) = cxx {
            cross_flags.push(format!("-DCMAKE_CXX_COMPILER={}", cxx));
        }
    }
    if target.contains("musl") || target.contains("linux") {
        cross_flags.push("-DCMAKE_SYSTEM_NAME=Linux".to_string());
        cross_flags.push(format!(
            "-DCMAKE_SYSTEM_PROCESSOR={}",
            if target.starts_with("x86_64") {
                "x86_64"
            } else if target.starts_with("aarch64") || target.starts_with("arm64") {
                "aarch64"
            } else {
                "x86_64"
            }
        ));
    }

    let mut cmake_args: Vec<&str> = vec![
        "-B",
        build_dir.to_str().unwrap(),
        "-S",
        src_dir.to_str().unwrap(),
        "-DCMAKE_BUILD_TYPE=Release",
        "-DNCNN_SHARED_LIB=OFF",
        "-DNCNN_BUILD_EXAMPLES=OFF",
        "-DNCNN_BUILD_TOOLS=OFF",
        "-DNCNN_BUILD_BENCHMARK=OFF",
        "-DNCNN_BUILD_TESTS=OFF",
        "-DNCNN_SIMPLEOCV=ON",
        "-DCMAKE_POLICY_VERSION_MINIMUM=3.5",
        &prefix_arg,
    ];
    for flag in &cross_flags {
        cmake_args.push(flag);
    }

    // Configure with CMake
    let status = Command::new("cmake")
        .args(&cmake_args)
        .status()
        .expect("failed to run cmake");
    assert!(status.success(), "cmake configure failed");

    // Build
    let jobs = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    let status = Command::new("cmake")
        .args([
            "--build",
            build_dir.to_str().unwrap(),
            "--target",
            "ncnn",
            "-j",
            &jobs.to_string(),
        ])
        .status()
        .expect("failed to build ncnn");
    assert!(status.success(), "ncnn build failed");

    // Install (just copy the .a)
    let lib_src = build_dir.join("src").join("libncnn.a");
    let lib_dst = install_dir.join("lib").join("libncnn.a");
    std::fs::create_dir_all(install_dir.join("lib")).unwrap();
    std::fs::copy(&lib_src, &lib_dst).unwrap();
}
