use rig::client::CompletionClient;
use rig::completion::Prompt;
use rig::completion::ToolDefinition;
use rig::providers::ollama;
use rig::tool::Tool;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::io::{self, Write};
use tokio::process::Command;

#[derive(Deserialize, Serialize, Debug)]
struct CmdArgs {
    command: String,
}

#[derive(Debug, Clone)]
struct CliExecutionTool;

impl Tool for CliExecutionTool {
    const NAME: &'static str = "execute_bash_command";
    type Error = rig::tool::ToolError;
    type Args = CmdArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description:
                "Executes local OS commands (like ls, pwd, echo) to check the system status."
                    .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "The command string to execute (e.g., 'ls', 'pwd')"
                    }
                },
                "required": ["command"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        println!("\n[Tool Executing] Executing command: {}", args.command);
        let output = if cfg!(target_os = "windows") {
            Command::new("cmd")
                .args(["/C", &args.command])
                .output()
                .await
        } else {
            Command::new("sh")
                .args(["-c", &args.command])
                .output()
                .await
        };

        match output {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout).to_string();
                let stderr = String::from_utf8_lossy(&out.stderr).to_string();
                if out.status.success() {
                    println!("[Tool Success] Execution was successful.");
                    Ok(stdout)
                } else {
                    println!("[Tool Error] Command returned an error.");
                    Ok(format!("Error: {}\nStderr: {}", stdout, stderr))
                }
            }
            Err(e) => {
                println!("[Tool Fatal] Failed to start the command itself.");
                Ok(format!("Failed to execute command: {}", e))
            }
        }
    }
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut model_name = "gemma4".to_string();

    if let Some(pos) = args.iter().position(|x| x == "-m") {
        if let Some(next_arg) = args.get(pos + 1) {
            model_name = next_arg.clone();
        } else {
            eprintln!("Error: Please specify a model name after -m.");
            std::process::exit(1);
        }
    }

    // Ollama client initialization
    let ollama_client =
        ollama::Client::new("http://localhost:11434").expect("Failed to initialize Ollama client");

    // CLI agent construction
    let agent = ollama_client
        .agent(&model_name) // Please adjust the model name as needed
        .preamble("
            You are an excellent CLI (Command Line) agent that supports the user's PC operations.
            Use the provided `execute_bash_command` tool appropriately to check the actual system status based on the user's instructions,
            and answer the user based on those results.
        ")
        .tool(CliExecutionTool)
        .build();

    println!("==================================================");
    println!("CLI Agent started. Please give me any instructions.");
    println!("   Type '/exit' to quit.");
    println!("==================================================");

    // Start of the interactive infinite loop
    loop {
        print!("\nUser>");
        // Flush stdout to display prompt immediately
        io::stdout().flush().expect("Failed to flush");

        let mut input = String::new();
        // Read user input
        io::stdin()
            .read_line(&mut input)
            .expect("Failed to read input");

        // Remove newline characters
        let user_prompt = input.trim();

        // Exit check
        if user_prompt.eq_ignore_ascii_case("/exit") {
            println!("Exiting. Thank you for using the agent!");
            break;
        }

        // Skip empty input
        if user_prompt.is_empty() {
            continue;
        }

        // Send instructions to AI
        let mut seconds = 0;
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(1));
        interval.tick().await;

        let prompt_fut = agent.prompt(user_prompt).into_future();
        tokio::pin!(prompt_fut);

        let response = loop {
            tokio::select! {
                res = &mut prompt_fut => {
                    seconds = 0;
                    let _ = seconds;
                    break res;
                }
                _ = interval.tick() => {
                    seconds += 1;
                    print!("\rAgent is thinking... ({}s)", seconds);
                    io::stdout().flush().expect("Failed to flush");
                }
            }
        };

        println!();

        match response {
            Ok(response) => {
                println!("\n--- AI Response ---");
                println!("{}", response);
            }
            Err(e) => {
                println!("\nError occurred: {}", e);
            }
        }
    }
}
