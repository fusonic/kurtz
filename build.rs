//! Pins the Tabler stylesheet. `assets/tabler.lock` holds the version and the
//! SRI hash. Without `assets/dist/tabler.min.css` (development) the admin loads
//! Tabler from the CDN with that hash as `integrity`. The Docker build downloads
//! the file; it is checked against the hash here and embedded in the binary.

use std::{env, fs, path::Path};

use base64::{engine::general_purpose::STANDARD, Engine};
use sha2::{Digest, Sha384};

fn main() {
    println!("cargo:rerun-if-changed=assets/tabler.lock");
    println!("cargo:rerun-if-changed=assets/dist/tabler.min.css");
    println!("cargo::rustc-check-cfg=cfg(tabler_embedded)");

    let lock = fs::read_to_string("assets/tabler.lock").expect("assets/tabler.lock is missing");
    let value = |key: &str| {
        lock.lines()
            .filter_map(|l| l.split_once('='))
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v.trim().to_string())
            .unwrap_or_else(|| panic!("{key} is missing in assets/tabler.lock"))
    };
    let (version, sri) = (value("TABLER_VERSION"), value("TABLER_SHA384"));

    let css = Path::new("assets/dist/tabler.min.css");
    if css.exists() {
        let actual = format!("sha384-{}", STANDARD.encode(Sha384::digest(fs::read(css).unwrap())));
        assert_eq!(
            actual, sri,
            "assets/dist/tabler.min.css does not match the hash in assets/tabler.lock"
        );
        println!("cargo:rustc-cfg=tabler_embedded");
    }

    let out = Path::new(&env::var("OUT_DIR").unwrap()).join("tabler.rs");
    fs::write(
        out,
        format!(
            "pub const TABLER_CDN_URL: &str = \"https://cdn.jsdelivr.net/npm/@tabler/core@{version}/dist/css/tabler.min.css\";\npub const TABLER_SRI: &str = \"{sri}\";\n"
        ),
    )
    .unwrap();
}
