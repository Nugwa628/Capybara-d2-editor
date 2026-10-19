//! Embeds the exe icon and the version information (Explorer's Properties >
//! Details) into `CapybarasD2Editor.exe`.
//!
//! Only a Windows host building for Windows can compile resources (rc.exe
//! from the Windows SDK, found by `winresource`). Anywhere else, or when
//! rc.exe is missing, the build goes on without them and Cargo prints a
//! warning.

const ICON: &str = "../../assets/branding/capybara.ico";
const PRODUCT: &str = "Capybara's D2 Editor";

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={ICON}");
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

#[cfg(not(windows))]
fn embed_resources() -> Result<(), String> {
    let _ = resource_script; // used by Windows hosts only
    Err("resources need a Windows host".to_owned())
}
