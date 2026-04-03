//! One-click install of NVIDIA runtime libraries for faster-whisper / CTranslate2 (CUDA 12 + cuDNN 9).
use crate::sidecar::default_python;
use std::fs;
use std::path::{Path, PathBuf};
use tauri::Manager;

const WIN_ARCHIVE_URL: &str =
    "https://github.com/Purfview/whisper-standalone-win/releases/download/libs/cuBLAS.and.cuDNN_CUDA12_win_v3.7z";

fn root_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("nvidia-whisper-gpu");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

#[cfg(target_os = "windows")]
pub fn windows_bin_dir(app: &tauri::AppHandle) -> Option<PathBuf> {
    let bin = root_dir(app).ok()?.join("cuda12-win").join("bin");
    if !bin.is_dir() {
        return None;
    }
    let ok = fs::read_dir(&bin).ok()?.flatten().any(|e| {
        let n = e.file_name().to_string_lossy().to_lowercase();
        n.ends_with(".dll") && (n.contains("cudnn") || n.contains("cublas"))
    });
    if ok {
        Some(bin)
    } else {
        None
    }
}

#[cfg(not(target_os = "windows"))]
pub fn unix_ld_library_path(app: &tauri::AppHandle) -> Option<String> {
    let p = root_dir(app).ok()?.join("linux-ld-library-path.txt");
    fs::read_to_string(p).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

fn count_win_gpu_dlls(bin: &Path) -> usize {
    let Ok(rd) = fs::read_dir(bin) else {
        return 0;
    };
    rd.flatten()
        .filter(|e| {
            let n = e.file_name().to_string_lossy().to_lowercase();
            n.ends_with(".dll") && (n.contains("cudnn") || n.contains("cublas"))
        })
        .count()
}

fn flatten_dlls(extract_root: &Path, bin: &Path) -> Result<usize, String> {
    fs::create_dir_all(bin).map_err(|e| e.to_string())?;
    let mut n = 0usize;
    fn walk(dir: &Path, bin: &Path, n: &mut usize) -> Result<(), String> {
        for e in fs::read_dir(dir).map_err(|e| e.to_string())? {
            let e = e.map_err(|e| e.to_string())?;
            let p = e.path();
            if p.is_dir() {
                walk(&p, bin, n)?;
            } else if p
                .extension()
                .and_then(|x| x.to_str())
                .map(|x| x.eq_ignore_ascii_case("dll"))
                .unwrap_or(false)
            {
                let name = p.file_name().ok_or_else(|| "dll path".to_string())?;
                let dest = bin.join(name);
                fs::copy(&p, &dest).map_err(|e| e.to_string())?;
                *n += 1;
            }
        }
        Ok(())
    }
    walk(extract_root, bin, &mut n)?;
    Ok(n)
}

fn download_to_url(url: &str, dest: &Path) -> Result<(), String> {
    let resp = ureq::get(url)
        .timeout(std::time::Duration::from_secs(7200))
        .call()
        .map_err(|e| format!("download failed: {e}"))?;
    let mut reader = resp.into_reader();
    let mut out = fs::File::create(dest).map_err(|e| e.to_string())?;
    std::io::copy(&mut reader, &mut out).map_err(|e| e.to_string())?;
    Ok(())
}

fn install_windows(app: &tauri::AppHandle) -> Result<String, String> {
    let root = root_dir(app)?;
    let cuda = root.join("cuda12-win");
    let bin = cuda.join("bin");
    let staging = cuda.join("staging");
    let archive = staging.join("cuBLAS.cuDNN_CUDA12_win_v3.7z");
    let extracted = staging.join("extracted");

    if bin.is_dir() && count_win_gpu_dlls(&bin) >= 2 {
        return Ok(
            "NVIDIA cuBLAS/cuDNN bundle is already installed for Yapper. Restart the engine if it was running."
                .into(),
        );
    }

    fs::create_dir_all(&staging).map_err(|e| e.to_string())?;
    if extracted.exists() {
        let _ = fs::remove_dir_all(&extracted);
    }
    if bin.exists() {
        let _ = fs::remove_dir_all(&bin);
    }
    fs::create_dir_all(&extracted).map_err(|e| e.to_string())?;

    let mut log = String::from("Downloading CUDA 12 libraries (~800 MB). This can take several minutes…\n");
    download_to_url(WIN_ARCHIVE_URL, &archive)?;
    log.push_str("Download finished. Extracting…\n");

    sevenz_rust::decompress_file(&archive, &extracted).map_err(|e| format!("extract 7z: {e}"))?;

    fs::create_dir_all(&bin).map_err(|e| e.to_string())?;
    let n = flatten_dlls(&extracted, &bin)?;
    if n == 0 {
        return Err(
            "Extracted archive contained no DLLs — check Purfview bundle layout or disk space.".into(),
        );
    }

    log.push_str(&format!(
        "Installed {n} DLLs into {}.\nRestart the inference engine (Home or Settings).",
        bin.display()
    ));
    Ok(log)
}

fn probe_linux_ld_path(py: &str) -> Result<String, String> {
    let script = r#"import os
import nvidia.cublas.lib
import nvidia.cudnn.lib
print(os.path.dirname(nvidia.cublas.lib.__file__) + ":" + os.path.dirname(nvidia.cudnn.lib.__file__))"#;
    let out = std::process::Command::new(py)
        .args(["-c", script])
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!(
            "Could not locate pip NVIDIA libs in Python: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn install_linux(app: &tauri::AppHandle) -> Result<String, String> {
    let py = default_python();
    let mut log = String::new();
    let pip = std::process::Command::new(&py)
        .args([
            "-m",
            "pip",
            "install",
            "--upgrade",
            "nvidia-cublas-cu12",
            "nvidia-cudnn-cu12>=9,<10",
        ])
        .output()
        .map_err(|e| e.to_string())?;
    log.push_str(&String::from_utf8_lossy(&pip.stdout));
    log.push_str(&String::from_utf8_lossy(&pip.stderr));
    if !pip.status.success() {
        return Err(format!("pip install failed:\n{log}"));
    }

    let ld = probe_linux_ld_path(&py)?;
    let root = root_dir(app)?;
    let marker = root.join("linux-ld-library-path.txt");
    fs::write(&marker, &ld).map_err(|e| e.to_string())?;
    log.push_str(&format!(
        "\nWrote LD_LIBRARY_PATH fragment to {}.\nRestart the inference engine.",
        marker.display()
    ));
    Ok(log)
}

pub fn install_blocking(app: &tauri::AppHandle) -> Result<String, String> {
    if cfg!(target_os = "macos") {
        return Err(
            "macOS: GPU packages from this installer target NVIDIA on Windows/Linux. Use Whisper on CPU, or set up your own Python environment."
                .into(),
        );
    }
    if cfg!(target_os = "windows") {
        install_windows(app)
    } else {
        install_linux(app)
    }
}
