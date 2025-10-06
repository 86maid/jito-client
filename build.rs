use rustc_version::{Version, version};

fn main() {
    if let Ok(v) = version() {
        if v >= Version::new(1, 85, 0) {
            println!("cargo::rustc-cfg=rustc_version_1_85_0");
        }
    }

    println!("cargo::rustc-check-cfg=cfg(rustc_version_1_85_0)");
}
