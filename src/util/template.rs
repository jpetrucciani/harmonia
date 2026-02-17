use std::fs;
use std::path::Path;

use crate::error::{HarmoniaError, Result};

pub fn render_template(template: &str, context: &serde_json::Value) -> Result<String> {
    let context = tera::Context::from_serialize(context)
        .map_err(|err| HarmoniaError::Other(anyhow::Error::new(err)))?;
    tera::Tera::one_off(template, &context, true)
        .map_err(|err| HarmoniaError::Other(anyhow::Error::new(err)))
}

pub fn render_template_file(path: &Path, context: &serde_json::Value) -> Result<String> {
    let template = fs::read_to_string(path)?;
    render_template(&template, context)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use serde_json::json;

    use crate::util::template::{render_template, render_template_file};

    #[test]
    fn renders_inline_template() {
        let output = render_template(
            "Hello {{ user }}. Repos: {% for repo in repos %}{{ repo }} {% endfor %}",
            &json!({
                "user": "harmonia",
                "repos": ["core", "app"],
            }),
        )
        .expect("render template");
        assert_eq!(output, "Hello harmonia. Repos: core app ");
    }

    #[test]
    fn renders_template_file() {
        let path = unique_temp_path("template-render");
        fs::write(&path, "MR {{ id }} for {{ repo }}").expect("write template file");
        let output = render_template_file(
            &path,
            &json!({
                "id": 42,
                "repo": "service",
            }),
        )
        .expect("render template file");
        assert_eq!(output, "MR 42 for service");
        let _ = fs::remove_file(&path);
    }

    fn unique_temp_path(prefix: &str) -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        let pid = std::process::id();
        std::env::temp_dir().join(format!("harmonia-{prefix}-{pid}-{nanos}.tmpl"))
    }
}
