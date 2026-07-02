//! Compile the C++/WinRT XAML-Islands shim with `cc` (MSVC) and link the WinRT umbrella
//! library. The Windows SDK ships the cppwinrt projection headers under
//! `Include\<ver>\cppwinrt`, which is NOT on the default INCLUDE path — we locate the newest
//! one and add it. Everything else (um/shared/ucrt/winrt) comes from `cc`'s MSVC environment.

use std::path::PathBuf;

fn main() {
    // Windows-only shim: on other hosts this crate is an empty stub (see src/lib.rs).
    if std::env::var("CARGO_CFG_WINDOWS").is_err() {
        return;
    }

    let cppwinrt = find_cppwinrt().expect(
        "Windows 10/11 SDK cppwinrt headers not found. Install the Windows SDK \
         (Visual Studio 'Desktop development with C++').",
    );

    let mut build = cc::Build::new();
    build
        .cpp(true)
        .std("c++17")
        .file("src/shim.cpp")
        .include(&cppwinrt)
        .flag("/EHsc") // C++/WinRT uses exceptions
        .flag("/bigobj") // the XAML cppwinrt headers blow past the default section limit
        .flag_if_supported("/permissive-");
    build.compile("daywinuishim");

    // WindowsApp.lib is the WinRT umbrella (RoInitialize, activation, XAML Islands).
    println!("cargo:rustc-link-lib=WindowsApp");
    println!("cargo:rustc-link-lib=user32");
    println!("cargo:rustc-link-lib=gdi32");
    println!("cargo:rerun-if-changed=src/shim.cpp");
    println!("cargo:rerun-if-changed=build.rs");
}

/// Newest `Windows Kits\10\Include\<ver>\cppwinrt` on the machine.
fn find_cppwinrt() -> Option<PathBuf> {
    let mut bases: Vec<PathBuf> = Vec::new();
    if let Ok(sdk) = std::env::var("WindowsSdkDir") {
        bases.push(PathBuf::from(sdk).join("Include"));
    }
    bases.push(PathBuf::from(
        r"C:\Program Files (x86)\Windows Kits\10\Include",
    ));
    bases.push(PathBuf::from(r"C:\Program Files\Windows Kits\10\Include"));

    let mut found: Vec<PathBuf> = Vec::new();
    for base in bases {
        let Ok(rd) = std::fs::read_dir(&base) else {
            continue;
        };
        for entry in rd.flatten() {
            let cppwinrt = entry.path().join("cppwinrt");
            if cppwinrt.join("winrt").join("base.h").exists() {
                found.push(cppwinrt);
            }
        }
    }
    found.sort();
    found.pop()
}
