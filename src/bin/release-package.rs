use std::env;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn main() -> Result<(), Box<dyn Error>> {
    let forward_args = env::args().skip(1).collect::<Vec<_>>();
    let explicit_target = extract_target_triple(&forward_args);
    let explicit_target_dir = extract_target_dir(&forward_args);

    run_release_build(&forward_args)?;

    let release_dir = release_dir(explicit_target.as_deref(), explicit_target_dir.as_deref());
    let (platform, arch) = platform_and_arch(explicit_target.as_deref());
    let ext = binary_extension(&platform);

    let app_name = env!("CARGO_PKG_NAME");
    let version = env!("CARGO_PKG_VERSION");

    let cli_source = release_dir.join(format!("operation-logger-cli{ext}"));
    let gui_source = release_dir.join(format!("operation-logger-gui{ext}"));

    let cli_target = release_dir.join(format!("{app_name}-cli-v{version}-{platform}-{arch}{ext}"));
    let gui_target = release_dir.join(format!("{app_name}-gui-v{version}-{platform}-{arch}{ext}"));

    copy_binary(&cli_source, &cli_target)?;
    copy_binary(&gui_source, &gui_target)?;

    println!("Created:");
    println!(" - {}", cli_target.display());
    println!(" - {}", gui_target.display());

    Ok(())
}

fn run_release_build(forward_args: &[String]) -> Result<(), Box<dyn Error>> {
    let status = Command::new("cargo")
        .arg("build")
        .arg("--release")
        .arg("--bin")
        .arg("operation-logger-cli")
        .arg("--bin")
        .arg("operation-logger-gui")
        .args(forward_args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()?;

    if !status.success() {
        return Err("cargo build failed".into());
    }

    Ok(())
}

fn extract_target_triple(args: &[String]) -> Option<String> {
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--target" {
            if let Some(value) = iter.next() {
                return Some(value.clone());
            }
            return None;
        }

        if let Some(value) = arg.strip_prefix("--target=") {
            return Some(value.to_string());
        }
    }

    None
}

fn extract_target_dir(args: &[String]) -> Option<PathBuf> {
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--target-dir" {
            if let Some(value) = iter.next() {
                return Some(PathBuf::from(value));
            }
            return None;
        }

        if let Some(value) = arg.strip_prefix("--target-dir=") {
            return Some(PathBuf::from(value));
        }
    }

    None
}

fn release_dir(target_triple: Option<&str>, explicit_target_dir: Option<&Path>) -> PathBuf {
    let target_dir = match explicit_target_dir {
        Some(path) => path.to_path_buf(),
        None => env::var_os("CARGO_TARGET_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("target")),
    };

    match target_triple {
        Some(triple) => target_dir.join(triple).join("release"),
        None => target_dir.join("release"),
    }
}

fn platform_and_arch(target_triple: Option<&str>) -> (String, String) {
    let default_platform = match env::consts::OS {
        "windows" => "windows",
        "linux" => "linux",
        "macos" => "macos",
        other => other,
    };

    let default_arch = match env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        "x86" => "x86",
        other => other,
    };

    let Some(triple) = target_triple else {
        return (default_platform.to_string(), default_arch.to_string());
    };

    let platform = if triple.contains("windows") {
        "windows".to_string()
    } else if triple.contains("linux") {
        "linux".to_string()
    } else if triple.contains("darwin") {
        "macos".to_string()
    } else {
        triple.split('-').nth(2).unwrap_or("unknown").to_string()
    };

    let raw_arch = triple.split('-').next().unwrap_or("unknown");
    let arch = match raw_arch {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        "i686" => "x86",
        _ => raw_arch,
    }
    .to_string();

    (platform, arch)
}

fn binary_extension(platform: &str) -> &'static str {
    if platform == "windows" { ".exe" } else { "" }
}

fn copy_binary(source: &Path, target: &Path) -> Result<(), Box<dyn Error>> {
    if !source.is_file() {
        return Err(format!("binary not found: {}", source.display()).into());
    }

    fs::copy(source, target)?;
    Ok(())
}
