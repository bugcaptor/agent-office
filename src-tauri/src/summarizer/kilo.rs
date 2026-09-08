/// Kilo's `models` command emits one `provider/model` identifier per line.
/// Keep only identifiers with both components so banner/log text cannot become
/// a model option when the CLI's output changes.
pub fn parse_models_stdout(stdout: &str) -> Vec<String> {
    let mut models = Vec::new();
    for line in stdout.lines() {
        let id = line.trim();
        let Some((provider, model)) = id.split_once('/') else {
            continue;
        };
        if provider.is_empty() || model.is_empty() || id.contains(char::is_whitespace) {
            continue;
        }
        if !models.iter().any(|known| known == id) {
            models.push(id.to_owned());
        }
    }
    models
}

fn models_command(program: &str) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(program);
    command.arg("models");
    command
}

/// Failures, timeouts, and output-contract changes yield an empty catalogue,
/// matching the other read-only automation model lookups.
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
    fn parses_unique_provider_model_ids_only() {
        assert_eq!(
            parse_models_stdout(
                "INFO loading\nkilo/openai/gpt-latest\nopenai/gpt-5\nopenai/gpt-5\ninvalid\n"
            ),
            vec!["kilo/openai/gpt-latest", "openai/gpt-5"]
        );
    }

    #[test]
    fn models_command_invokes_kilo_models_directly() {
        let command = models_command("kilo");
        assert_eq!(command.as_std().get_program(), "kilo");
        let args: Vec<_> = command
            .as_std()
            .get_args()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect();
        assert_eq!(args, vec!["models"]);
    }
}
