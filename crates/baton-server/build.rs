use std::{
    env, fs,
    path::{Path, PathBuf},
};

fn collect(root: &Path, dir: &Path, entries: &mut Vec<(String, PathBuf)>) {
    for entry in fs::read_dir(dir).expect("read web build directory") {
        let path = entry.expect("read web build entry").path();
        if path.is_dir() {
            collect(root, &path, entries);
        } else {
            let name = path
                .strip_prefix(root)
                .unwrap()
                .to_str()
                .expect("UTF-8 web asset path")
                .replace('\\', "/");
            entries.push((name, path.canonicalize().unwrap()));
        }
    }
}

fn main() {
    println!("cargo:rerun-if-env-changed=BATON_WEB_DIR");
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let mut entries = Vec::new();
    if let Some(dir) = env::var_os("BATON_WEB_DIR") {
        let dir = PathBuf::from(dir);
        assert!(
            dir.join("index.html").is_file(),
            "BATON_WEB_DIR must contain a built web app"
        );
        println!("cargo:rerun-if-changed={}", dir.display());
        collect(&dir, &dir, &mut entries);
    } else {
        let placeholder = out.join("index.html");
        fs::write(&placeholder, "<!doctype html><title>Baton</title><p>Web UI not embedded. Build the UI and set BATON_WEB_DIR when building Baton, or run the Vite development server.</p>").unwrap();
        entries.push(("index.html".into(), placeholder));
    }
    entries.sort();
    let mut code = String::from(
        "fn embedded_asset(path: &str) -> Option<&'static [u8]> {\n    match path {\n",
    );
    for (name, path) in entries {
        code.push_str(&format!(
            "        {:?} => Some(include_bytes!({:?})),\n",
            name, path
        ));
    }
    code.push_str("        _ => None,\n    }\n}\n");
    fs::write(out.join("web_assets.rs"), code).unwrap();
}
