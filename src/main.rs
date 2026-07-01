use rig::client::CompletionClient;
use rig::completion::{Chat, Message, ToolDefinition};
use rig::providers::ollama;
use rig::tool::Tool;
use scraper::{Html, Selector};
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
            description: "Executes local OS commands (like ls, pwd, echo).".to_string(),
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

#[derive(Deserialize, Serialize, Debug)]
struct SearchArgs {
    query: String,
}

#[derive(Debug, Clone)]
struct DuckDuckGoSearchTool;

impl Tool for DuckDuckGoSearchTool {
    const NAME: &'static str = "web_search";
    type Error = rig::tool::ToolError;
    type Args = SearchArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Search the internet for the latest information.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Search keywords (e.g., 'how to use Rust rig framework', 'latest news')"
                    }
                },
                "required": ["query"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        println!(
            "\n[Tool Executing] Searching the web with DuckDuckGo: {}",
            args.query
        );

        let url = "https://lite.duckduckgo.com/lite/";

        let client = reqwest::Client::builder()
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
            .build()
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

        let res = client
            .post(url)
            .form(&[("q", &args.query)])
            .send()
            .await
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

        let html_content = res
            .text()
            .await
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

        let document = Html::parse_document(&html_content);

        let tr_selector = Selector::parse("table tr").unwrap();
        let link_selector = Selector::parse("a.result-link").unwrap();
        let snippet_selector = Selector::parse("td.result-snippet").unwrap();

        let mut summary = String::new();
        let mut count = 0;

        let rows: Vec<_> = document.select(&tr_selector).collect();

        for (i, row) in rows.iter().enumerate() {
            if count >= 3 {
                break;
            }

            if let Some(link_elem) = row.select(&link_selector).next() {
                let title = link_elem
                    .text()
                    .collect::<Vec<_>>()
                    .join(" ")
                    .trim()
                    .to_string();
                let url = link_elem.value().attr("href").unwrap_or("").to_string();

                let mut snippet = String::new();
                if let Some(next_row) = rows.get(i + 1) {
                    if let Some(snip_elem) = next_row.select(&snippet_selector).next() {
                        snippet = snip_elem
                            .text()
                            .collect::<Vec<_>>()
                            .join(" ")
                            .trim()
                            .to_string();
                    }
                }

                summary.push_str(&format!(
                    "- Title: {}\n  URL: {}\n  Snippet: {}\n\n",
                    title, url, snippet
                ));
                count += 1;
            }
        }

        if summary.is_empty() {
            Ok("No search results were found, or they were blocked.".to_string())
        } else {
            Ok(summary)
        }
    }
}

#[derive(Deserialize, Serialize, Debug)]
struct FetchArgs {
    url: String,
}

#[derive(Debug, Clone)]
struct FetchWebPageTool;

impl Tool for FetchWebPageTool {
    const NAME: &'static str = "fetch_web_page";
    type Error = rig::tool::ToolError;
    type Args = FetchArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Accesses the webpage at the specified URL and retrieves its content (body text). This is also used when you want to view the detailed content of a URL found via `web_search`.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "The absolute URL of the web page to access."
                    }
                },
                "required": ["url"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        println!("\n[Tool Executing] Retrieving web page: {}", args.url);

        let client = reqwest::Client::builder()
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
            .build()
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

        let res = client
            .get(&args.url)
            .send()
            .await
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

        let html_content = res
            .text()
            .await
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

        let document = Html::parse_document(&html_content);

        let body_selector = Selector::parse("body").unwrap();

        let mut text_content = String::new();
        if let Some(body) = document.select(&body_selector).next() {
            for text in body.text() {
                let trimmed = text.trim();
                if !trimmed.is_empty() && trimmed.len() > 1 {
                    text_content.push_str(trimmed);
                    text_content.push(' ');
                }
            }
        }

        if text_content.len() > 20000 {
            println!("\nOmitted due to character limit");
            text_content.truncate(20000);
            text_content.push_str("\n...[Omitted due to character limit]...");
        }

        if text_content.is_empty() {
            Ok("Could not retrieve the text from the web page.".to_string())
        } else {
            Ok(text_content)
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
            You are an excellent CLI & Web agent.
            - Use `execute_bash_command` to check local system status or run commands.
            - Use `web_search` to find latest information, documentation, or generic knowledge from the internet.
            Choose the best tool based on the user's request.
        ")
        .tool(CliExecutionTool { seconds: Arc::clone(&shared_seconds) })
        .tool(DuckDuckGoSearchTool)
        .tool(FetchWebPageTool)
        .default_max_turns(10)
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
