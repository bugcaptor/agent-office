/// `pi --list-models` prints provider/model columns followed by capacity and
/// capability columns. Require its header and row shape to exclude diagnostics.
pub fn parse_models_stdout(stdout: &str) -> Vec<String> {
    let mut models = Vec::new();
    let mut in_table = false;
    for line in stdout.lines() {
        let columns: Vec<_> = line.split_whitespace().collect();
        if columns
            == [
                "provider", "model", "context", "max-out", "thinking", "images",
            ]
        {
            in_table = true;
            continue;
        }
        if !in_table || columns.len() != 6 {
            continue;
        }
        if !matches!(columns[4], "yes" | "no") || !matches!(columns[5], "yes" | "no") {
            continue;
        }
        if !columns[2..4].iter().all(|capacity| {
            capacity.starts_with(|c: char| c.is_ascii_digit())
                && capacity
                    .chars()
                    .all(|c| c.is_ascii_digit() || matches!(c, '.' | 'K' | 'M' | 'G'))
        }) {
            continue;
        }
        let id = format!("{}/{}", columns[0], columns[1]);
        if !models.contains(&id) {
            models.push(id);
        }
    }
    models
}

fn models_command(program: &str) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(program);
    command.arg("--list-models");
    command
}

pub async fn list_models(timeout: std::time::Duration, program: &str) -> Vec<String> {
    let mut command = models_command(program);
    let Some(stdout) = super::model_catalog_stdout(&mut command, timeout).await else {
        return Vec::new();
    };
    parse_models_stdout(&String::from_utf8_lossy(&stdout))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_provider_and_nested_model_ids_without_diagnostics() {
        let stdout = "Warning: invalid settings file\n\
            provider model context max-out thinking images\n\
            anthropic claude-sonnet-4-5 200K 64K yes yes\n\
            openrouter vendor/model:free 1.0M 32.8K no yes\n\
            anthropic claude-sonnet-4-5 200K 64K yes yes\n\
            Warning: invalid capacity values yes no\n\
            incomplete model 200K\n";
        assert_eq!(
            parse_models_stdout(stdout),
            vec![
                "anthropic/claude-sonnet-4-5",
                "openrouter/vendor/model:free"
            ]
        );
    }

    #[test]
    fn missing_header_and_empty_catalogue_are_empty() {
        assert!(parse_models_stdout("No models available").is_empty());
        assert!(parse_models_stdout("provider model context max-out thinking images\n").is_empty());
        assert!(parse_models_stdout("anthropic sonnet 200K 64K yes yes").is_empty());
    }

    #[test]
    fn invokes_read_only_model_list() {
        let command = models_command("pi");
        assert_eq!(command.as_std().get_program(), "pi");
        assert_eq!(
            command.as_std().get_args().collect::<Vec<_>>(),
            vec!["--list-models"]
        );
    }
}
