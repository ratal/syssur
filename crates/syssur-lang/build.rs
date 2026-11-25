use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn collect_library_files(root: &Path, dir: &Path, out: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            collect_library_files(root, &path, out);
        } else if matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("sysml" | "kerml")
        ) {
            if let Ok(rel) = path.strip_prefix(root) {
                let rel_str = rel.to_string_lossy().replace('\\', "/");
                out.push(rel_str);
            }
        }
    }
}

fn main() {
    println!("cargo:rerun-if-changed=stdlib");

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let stdlib_dir = manifest_dir.join("stdlib");
    let mut files = Vec::new();
    collect_library_files(&stdlib_dir, &stdlib_dir, &mut files);
    files.sort();

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let mut code = String::from("pub const STDLIB_SOURCES: &[(&str, &str)] = &[\n");
    for rel in files {
        let escaped_rel = rel.escape_default();
        code.push_str(&format!(
            "    (\"{escaped_rel}\", include_str!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/stdlib/{escaped_rel}\"))),\n"
        ));
    }
    code.push_str("];\n");
    fs::write(out_dir.join("stdlib_sources.rs"), code).expect("write stdlib_sources.rs");
}
