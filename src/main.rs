use rig::client::CompletionClient;
use rig::completion::{Chat, Message, ToolDefinition};
use rig::providers::ollama;
use rig::tool::Tool;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::future::IntoFuture;
use std::io::{self, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::process::Command;

#[derive(Deserialize, Serialize, Debug)]
struct CmdArgs {
    command: String,
}

#[derive(Debug, Clone)]
struct CliExecutionTool {
    seconds: Arc<AtomicU64>,
}

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
        self.seconds.store(0, Ordering::SeqCst);

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

                self.seconds.store(0, Ordering::SeqCst);

                if out.status.success() {
                    println!("[Tool Success] Execution was successful.");
                    Ok(stdout)
                } else {
                    println!("[Tool Error] Command returned an error.");
                    Ok(format!("Error: {}\nStderr: {}", stdout, stderr))
                }
            }
            Err(e) => {
                self.seconds.store(0, Ordering::SeqCst);
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

    let shared_seconds = Arc::new(AtomicU64::new(0));
    let ollama_client =
        ollama::Client::new("http://localhost:11434").expect("Failed to initialize Ollama client");

    let mut conversation_history: Vec<Message> = vec![];

    let agent = ollama_client
        .agent(&model_name)
        .preamble("
            You are an excellent CLI (Command Line) agent that supports the user's PC operations.
            Use the provided `execute_bash_command` tool appropriately to check the actual system status based on the user's instructions,
            and answer the user based on those results.
        ")
        .tool(CliExecutionTool { seconds: Arc::clone(&shared_seconds) })
        .build();

    println!("==================================================");
    println!("CLI Agent started. Please give me any instructions.");
    println!("   Type '/exit' to quit.");
    println!("==================================================");

    loop {
        print!("\nUser>");
        io::stdout().flush().expect("Failed to flush");

        let mut input = String::new();
        io::stdin()
            .read_line(&mut input)
            .expect("Failed to read input");

        let user_prompt = input.trim();

        if user_prompt.eq_ignore_ascii_case("/exit") {
            println!("Exiting. Thank you for using the agent!");
            break;
        }

        if user_prompt.is_empty() {
            continue;
        }

        shared_seconds.store(0, Ordering::SeqCst);
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(1));
        interval.tick().await;

        print!("\rAgent is thinking... (0s)");
        io::stdout().flush().expect("Failed to flush");

        let chat_fut = agent
            .chat(user_prompt, &mut conversation_history)
            .into_future();
        tokio::pin!(chat_fut);

        let response = loop {
            tokio::select! {
                res = &mut chat_fut => {
                    break res;
                }
                _ = interval.tick() => {
                    let current = shared_seconds.fetch_add(1, Ordering::SeqCst) + 1;
                    print!("\rAgent is thinking... ({}s)", current);
                    io::stdout().flush().expect("Failed to flush");
                }
            }
        };

        match response {
            Ok(response_text) => {
                println!("\n--- AI Response ---");
                println!("{}", response_text);
            }
            Err(e) => {
                println!("\nError occurred: {}", e);
            }
        }
    }
}
