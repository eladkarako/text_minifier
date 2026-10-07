use std::{env, path::PathBuf};
use winres::WindowsResource;

fn main() {
    let target_os =
        env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os != "windows" {
        return;
    }

    let manifest_dir: PathBuf = env::var("CARGO_MANIFEST_DIR")
        .expect("CARGO_MANIFEST_DIR not set")
        .into();

    let cargo_toml = load_cargo_toml(&manifest_dir);
    let package = &cargo_toml["package"];

    let name = get_string(package, "name", "unknown");
    let version = get_string(package, "version", "0.0.0");
    let description = get_string(package, "description", "");
    let repository = get_string(package, "repository", "");
    let author = get_first_author(package);

    let (major, minor, patch, release) = parse_version(version);
    let packed = pack_version(major, minor, patch, release);

    set_windows_resources(
        &manifest_dir,
        name,
        description,
        repository,
        author,
        major,
        minor,
        patch,
        release,
        packed,
    );
}

/// Loads and parses the Cargo.toml file
fn load_cargo_toml(manifest_dir: &PathBuf) -> toml::Value {
    let cargo_toml_path = manifest_dir.join("Cargo.toml");
    let content = std::fs::read_to_string(&cargo_toml_path)
        .expect("Failed to read Cargo.toml");
    toml::from_str(&content).expect("Failed to parse Cargo.toml")
}

/// Retrieves a string value from a TOML table with a default fallback
#[inline]
fn get_string<'a>(
    table: &'a toml::Value,
    key: &str,
    default: &'a str,
) -> &'a str {
    table[key].as_str().unwrap_or(default)
}

/// Extracts the first author from the authors array
fn get_first_author(package: &toml::Value) -> &str {
    package["authors"]
        .as_array()
        .and_then(|arr| arr.first())
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown")
}

/// Parses a version string (e.g., "1.2.3.4") into (major, minor, patch, release)
fn parse_version(version: &str) -> (u64, u64, u64, u64) {
    let parts: Vec<&str> = version.split('.').collect();

    let major =
        parts.get(0).and_then(|v| v.parse().ok()).unwrap_or(0);
    let minor =
        parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(0);
    let patch =
        parts.get(2).and_then(|v| v.parse().ok()).unwrap_or(0);
    let release =
        parts.get(3).and_then(|v| v.parse().ok()).unwrap_or(0);

    (major, minor, patch, release)
}

/// Packs version numbers into a single u64 (48-bit major, 32-bit minor, 16-bit patch, 16-bit release)
#[inline]
fn pack_version(
    major: u64,
    minor: u64,
    patch: u64,
    release: u64,
) -> u64 {
    (major << 48) | (minor << 32) | (patch << 16) | release
}

/// Configures and compiles Windows resource information
fn set_windows_resources(
    manifest_dir: &PathBuf,
    name: &str,
    description: &str,
    repository: &str,
    author: &str,
    major: u64,
    minor: u64,
    patch: u64,
    release: u64,
    packed: u64,
) {
    let path_icon =
        manifest_dir.join("resources").join("logos").join("app.ico");
    let path_manifest =
        manifest_dir.join("resources").join("app.manifest");

    let mut res = WindowsResource::new();
    res.set_icon(path_icon.to_string_lossy().as_ref());
    res.set_manifest_file(path_manifest.to_string_lossy().as_ref());

    // Set language to English (US)
    let lang_english: u16 = 0x09;
    let sublang_english_us: u16 = 0x01;
    let langid: u16 = (sublang_english_us << 10) | lang_english;
    res.set_language(langid);

    // Set metadata
    res.set("Comments", repository);
    res.set("FileDescription", description);
    res.set("InternalName", &format!("{}.exe", name));
    res.set("OriginalFilename", &format!("{}.exe", name));
    res.set("CompanyName", author);
    res.set("LegalCopyright", &format!("{}/LICENSE", repository));
    res.set("ProductName", name);

    // Set version strings
    let version_string =
        format!("{}.{}.{}.{}", major, minor, patch, release);
    res.set("FileVersion", &version_string);
    res.set("ProductVersion", &version_string);
    res.set_version_info(winres::VersionInfo::FILEVERSION, packed);
    res.set_version_info(
        winres::VersionInfo::PRODUCTVERSION,
        packed,
    );

    res.compile().expect("Failed to compile Windows resources");
}
