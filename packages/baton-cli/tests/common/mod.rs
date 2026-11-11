use serde_json::{Value, json};
use std::{fs, path::Path};

pub fn write_config(root: &Path, config: &Value) {
    let baton = root.join(".baton");
    fs::create_dir_all(&baton).unwrap();
    let metadata =
        json!({"schemaVersion": config["schemaVersion"], "projectId": config.get("projectId")});
    let mut metadata = metadata;
    if metadata["projectId"].is_null() {
        metadata.as_object_mut().unwrap().remove("projectId");
    }
    fs::write(baton.join("project.yml"), metadata.to_string()).unwrap();
    for kind in ["agents", "pipelines"] {
        let directory = baton.join(kind);
        if directory.exists() {
            fs::remove_dir_all(&directory).unwrap();
        }
        fs::create_dir_all(&directory).unwrap();
        for item in config[kind].as_array().unwrap() {
            fs::write(
                directory.join(format!("{}.yml", item["id"].as_str().unwrap())),
                item.to_string(),
            )
            .unwrap();
        }
    }
}

#[allow(dead_code)]
pub fn read_config(root: &Path) -> Value {
    let baton = root.join(".baton");
    let mut config: Value =
        serde_yaml_ng::from_str(&fs::read_to_string(baton.join("project.yml")).unwrap()).unwrap();
    for kind in ["agents", "pipelines"] {
        let mut entries: Vec<_> = fs::read_dir(baton.join(kind))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        entries.sort();
        config[kind] = Value::Array(
            entries
                .iter()
                .map(|path| serde_yaml_ng::from_str(&fs::read_to_string(path).unwrap()).unwrap())
                .collect(),
        );
    }
    config
}
