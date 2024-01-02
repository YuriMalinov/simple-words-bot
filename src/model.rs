use std::{fs, path::Path};

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Task {
    pub id: u64,
    pub sentence: String,
    pub masked_sentence: String,
    pub correct: String,
    pub base: String,
    pub sentence_ru: String,
    pub sentence_en: String,
    pub hints: Vec<Hint>,
    pub wrong_answers: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct Hint {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Deserialize)]
pub struct TaskGroup {
    pub theme: String,
    pub category: String,
    pub tasks: Vec<Task>,
}

fn read_model_from_file(file_path: &str) -> anyhow::Result<TaskGroup> {
    let file_contents = std::fs::read_to_string(file_path)?;
    let model: TaskGroup = serde_yaml::from_str(&file_contents)?;
    Ok(model)
}

pub fn scan_data_directory(directory_path: &str) -> anyhow::Result<Vec<TaskGroup>> {
    let mut task_groups = Vec::new();
    let path = Path::new(directory_path);
    if path.is_dir() {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let file_path = entry.path();
            if let Some(extension) = file_path.extension() {
                if extension == "yaml" || extension == "yml" {
                    if let Some(file_path_str) = file_path.to_str() {
                        if let Ok(task_group) = read_model_from_file(file_path_str) {
                            task_groups.push(task_group);
                        }
                    }
                }
            }
        }
    }
    Ok(task_groups)
}
