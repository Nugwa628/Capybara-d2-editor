//! Embeds the exe icon and the version information (Explorer's Properties >
//! Details) into `CapybarasD2Editor.exe`.
//!
//! A Windows host uses rc.exe from the Windows SDK, found by `winresource`.
//! A Linux host cross-building the exe (`scripts/build-release.sh --windows`)
//! uses the first of: `CAPY_RC`, an rc.exe-compatible command such as
//! `zig rc` or `llvm-rc` (its `.res` is linked by lld); `WINDRES` or
//! `x86_64-w64-mingw32-windres` for a `-gnu` target (a COFF object); and
//! `llvm-rc` for an `-msvc` target. When no tool works the build goes on
//! without the resources and Cargo prints a warning.

const ICON: &str = "../../assets/branding/capybara.ico";
const PRODUCT: &str = "Capybara's D2 Editor";

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={ICON}");
    println!("cargo:rerun-if-env-changed=CAPY_RC");
    println!("cargo:rerun-if-env-changed=WINDRES");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    if let Err(e) = embed_resources() {
        println!("cargo:warning=no icon or version info in the exe: {e}");
    }
}

/// The resource script. It is written here rather than by `winresource`
/// because that crate escapes `'` as `\'`, which rc.exe keeps literally.
fn resource_script(icon: &std::path::Path) -> String {
    let env = |k: &str| std::env::var(k).unwrap_or_default();
    let num = |k: &str| env(k).parse::<u16>().unwrap_or(0);
    let (major, minor, patch) = (
        num("CARGO_PKG_VERSION_MAJOR"),
        num("CARGO_PKG_VERSION_MINOR"),
        num("CARGO_PKG_VERSION_PATCH"),
    );
    let version = env("CARGO_PKG_VERSION");
    // RC strings: backslash escapes, `""` is a quote.
    let quote = |s: &str| s.replace('\\', "\\\\").replace('"', "\"\"");
    let values = [
        ("CompanyName", PRODUCT.to_owned()),
        ("FileDescription", PRODUCT.to_owned()),
        ("FileVersion", version.clone()),
        ("InternalName", "CapybarasD2Editor".to_owned()),
        ("LegalCopyright", "MIT License".to_owned()),
        ("OriginalFilename", "CapybarasD2Editor.exe".to_owned()),
        ("ProductName", PRODUCT.to_owned()),
        ("ProductVersion", version),
    ];
    let mut rc = String::new();
    rc.push_str("#pragma code_page(65001)\n");
    rc.push_str(&format!(
        "1 ICON \"{}\"\n",
        quote(&icon.display().to_string())
    ));
    rc.push_str(&format!(
        "1 VERSIONINFO\nFILEVERSION {major},{minor},{patch},0\nPRODUCTVERSION {major},{minor},{patch},0\n\
         FILEFLAGSMASK 0x3f\nFILEFLAGS 0x0\nFILEOS 0x40004\nFILETYPE 0x1\nFILESUBTYPE 0x0\n"
    ));
    rc.push_str("{\nBLOCK \"StringFileInfo\"\n{\nBLOCK \"040904b0\"\n{\n");
    for (k, v) in values {
        rc.push_str(&format!("VALUE \"{k}\", \"{}\"\n", quote(&v)));
    }
    rc.push_str("}\n}\nBLOCK \"VarFileInfo\"\n{\nVALUE \"Translation\", 0x409, 1200\n}\n}\n");
    rc
}

#[cfg(windows)]
fn embed_resources() -> Result<(), String> {
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").map_err(|e| e.to_string())?);
    let manifest = std::env::var("CARGO_MANIFEST_DIR").map_err(|e| e.to_string())?;
    let icon = std::path::Path::new(&manifest).join(ICON);
    if !icon.is_file() {
        return Err(format!("{} is missing", icon.display()));
    }
    let rc = out.join("capybara.rc");
    std::fs::write(&rc, resource_script(&icon)).map_err(|e| e.to_string())?;
    winresource::WindowsResource::new()
        .set_resource_file(&rc.display().to_string())
        .compile()
        .map_err(|e| e.to_string())
}

/// Cross build from a non-Windows host: compile the script with the first
/// resource compiler that works and hand the result to the linker.
#[cfg(not(windows))]
fn embed_resources() -> Result<(), String> {
    use std::path::{Path, PathBuf};
    use std::process::Command;

    let out = PathBuf::from(std::env::var("OUT_DIR").map_err(|e| e.to_string())?);
    let manifest = std::env::var("CARGO_MANIFEST_DIR").map_err(|e| e.to_string())?;
    let icon = Path::new(&manifest).join(ICON);
    if !icon.is_file() {
        return Err(format!("{} is missing", icon.display()));
    }
    let rc = out.join("capybara.rc");
    std::fs::write(&rc, resource_script(&icon)).map_err(|e| e.to_string())?;
    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();

    // (program and leading arguments, produces a COFF object instead of .res)
    let mut tools: Vec<(Vec<String>, bool)> = Vec::new();
    if let Ok(cmd) = std::env::var("CAPY_RC") {
        tools.push((cmd.split_whitespace().map(str::to_owned).collect(), false));
    }
    if target_env == "gnu" {
        let windres =
            std::env::var("WINDRES").unwrap_or_else(|_| format!("{arch}-w64-mingw32-windres"));
        tools.push((vec![windres], true));
    }
    if target_env == "msvc" {
        tools.push((vec!["llvm-rc".to_owned()], false));
    }

    let mut tried = Vec::new();
    for (cmd, coff) in tools {
        let Some((program, lead)) = cmd.split_first() else {
            continue;
        };
        // Run in OUT_DIR with relative names: an absolute Unix path starts
        // with `/`, which rc.exe-style tools read as an option.
        let mut c = Command::new(program);
        c.current_dir(&out).args(lead);
        let name = if coff {
            let bfd = if arch == "x86_64" {
                "pe-x86-64"
            } else {
                "pe-i386"
            };
            c.args(["--target", bfd, "--input-format=rc", "--output-format=coff"])
                .args(["--input", "capybara.rc", "--output", "capybara-res.o"]);
            "capybara-res.o"
        } else {
            c.args(["/fo", "capybara.res", "capybara.rc"]);
            "capybara.res"
        };
        let output = out.join(name);
        let _ = std::fs::remove_file(&output);
        match c.status() {
            Ok(s) if s.success() && output.is_file() => {
                println!("cargo:rustc-link-arg-bins={}", output.display());
                return Ok(());
            }
            Ok(s) => tried.push(format!("{} ({s})", cmd.join(" "))),
            Err(e) => tried.push(format!("{} ({e})", cmd.join(" "))),
        }
    }
    if tried.is_empty() {
        return Err("no resource compiler for this target; set CAPY_RC".to_owned());
    }
    Err(format!("no resource compiler worked: {}", tried.join(", ")))
}
