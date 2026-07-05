use std::collections::BTreeMap;

use anyhow::{Result, bail};

#[derive(Debug, Clone, Default)]
pub struct Variables {
    values: BTreeMap<String, String>,
}

impl Variables {
    pub fn insert(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.values.insert(key.into(), value.into());
    }

    pub fn render(&self, input: &str) -> Result<String> {
        let mut current = input.to_owned();
        for _ in 0..32 {
            let next = self.render_once(&current)?;
            if next == current {
                return Ok(next);
            }
            current = next;
        }
        bail!("template expansion exceeded 32 passes (possible variable cycle): {input}")
    }

    fn render_once(&self, input: &str) -> Result<String> {
        let mut output = String::with_capacity(input.len());
        let mut rest = input;
        while let Some(start) = rest.find('{') {
            output.push_str(&rest[..start]);
            let after_open = &rest[start + 1..];
            let Some(end) = after_open.find('}') else {
                bail!("unclosed template variable in: {input}");
            };
            let key = &after_open[..end];
            if key.is_empty() {
                bail!("empty template variable in: {input}");
            }
            let value = self
                .values
                .get(key)
                .ok_or_else(|| anyhow::anyhow!("unknown template variable {{{key}}}"))?;
            output.push_str(value);
            rest = &after_open[end + 1..];
        }
        output.push_str(rest);
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_multiple_variables() {
        let mut variables = Variables::default();
        variables.insert("repo", "v8");
        variables.insert("jobs", "8");
        assert_eq!(
            variables.render("build {repo} with {jobs}").unwrap(),
            "build v8 with 8"
        );
    }

    #[test]
    fn rejects_unknown_variables() {
        assert!(Variables::default().render("{missing}").is_err());
    }

    #[test]
    fn recursively_renders_variable_values() {
        let mut variables = Variables::default();
        variables.insert("root", "/tmp/repo");
        variables.insert("build", "{root}/build");
        assert_eq!(
            variables.render("{build}/file").unwrap(),
            "/tmp/repo/build/file"
        );
    }

    #[test]
    fn rejects_variable_cycles() {
        let mut variables = Variables::default();
        variables.insert("a", "{b}");
        variables.insert("b", "{a}");
        assert!(variables.render("{a}").is_err());
    }
}
