use futures::StreamExt;
use rig::client::CompletionClient;
use rig::completion::{Message, ToolDefinition};
use rig::providers::ollama;
use rig::streaming::StreamingChat;
use rig::tool::Tool;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tokio::process::Command;

#[derive(Deserialize, Serialize, Debug)]
struct CmdArgs {
    command: String,
}

#[derive(Debug, Clone)]
struct CliExecutionTool {
    seconds: Arc<AtomicU64>,
    is_tool_running: Arc<AtomicBool>,
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
        self.is_tool_running.store(true, Ordering::SeqCst);
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

        let res = match output {
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
        };
        self.is_tool_running.store(false, Ordering::SeqCst);
        res
    }
}

#[derive(Deserialize, Serialize, Debug)]
struct SearchArgs {
    query: String,
}

#[derive(Debug, Clone)]
struct DuckDuckGoSearchTool {
    seconds: Arc<AtomicU64>,
    is_tool_running: Arc<AtomicBool>,
}

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
        self.is_tool_running.store(true, Ordering::SeqCst);
        self.seconds.store(0, Ordering::SeqCst);

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
        let res = if summary.is_empty() {
            Ok("No search results were found, or they were blocked.".to_string())
        } else {
            Ok(summary)
        };
        self.is_tool_running.store(false, Ordering::SeqCst);
        res
    }
}

#[derive(Deserialize, Serialize, Debug)]
struct FetchArgs {
    url: String,
}

#[derive(Debug, Clone)]
struct FetchWebPageTool {
    seconds: Arc<AtomicU64>,
    is_tool_running: Arc<AtomicBool>,
}

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
        self.is_tool_running.store(true, Ordering::SeqCst);
        self.seconds.store(0, Ordering::SeqCst);

        println!("\n[Tool Executing] Fetching a web page: {}", args.url);

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

        let mut text_content = String::new();

        let selectors = vec!["main", "article", "body"];
        let mut target_element = None;

        for sel in selectors {
            if let Ok(selector) = Selector::parse(sel) {
                if let Some(elem) = document.select(&selector).next() {
                    target_element = Some(elem);
                    break;
                }
            }
        }

        if let Some(body) = target_element {
            for node in body.descendants() {
                if let Some(element) = node.value().as_element() {
                    match element.name() {
                        "script" | "style" | "nav" | "footer" | "header" | "iframe"
                        | "noscript" => continue,
                        _ => {}
                    }
                }
                if let Some(text) = node.value().as_text() {
                    let trimmed = text.trim();
                    if !trimmed.is_empty() && trimmed.len() > 1 {
                        text_content.push_str(trimmed);
                        text_content.push(' ');
                    }
                }
            }
        }

        if text_content.len() > 20000 {
            let mut limit = 20000;
            while !text_content.is_char_boundary(limit) {
                limit -= 1;
            }
            text_content.truncate(limit);
            text_content.push_str("\n...[Omitted due to character limit]...");
        }

        let res = if text_content.is_empty() {
            Ok("Could not retrieve relevant main text from the web page.".to_string())
        } else {
            Ok(text_content)
        };
        self.is_tool_running.store(false, Ordering::SeqCst);
        res
    }
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut model_name = "gemma4".to_string();
    let mut endpoint = "http://localhost:11434".to_string();

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-m" => {
                if let Some(next_arg) = args.get(i + 1) {
                    model_name = next_arg.clone();
                    i += 2;
                } else {
                    eprintln!("Error: Please specify a model name after -m.");
                    std::process::exit(1);
                }
            }
            "-e" => {
                if let Some(next_arg) = args.get(i + 1) {
                    endpoint = next_arg.clone();
                    i += 2;
                } else {
                    eprintln!("Error: Please specify an endpoint URL after -e.");
                    std::process::exit(1);
                }
            }
            _ => {
                i += 1;
            }
        }
    }

    println!("==================================================");
    println!("Ollama Host : {}", endpoint);
    println!("Active Model: {}", model_name);
    println!("CLI Agent started. Please give me any instructions.");
    println!("   Type '/exit' to quit.");
    println!("==================================================");

    let shared_seconds = Arc::new(AtomicU64::new(0));
    let is_tool_running = Arc::new(AtomicBool::new(false));

    let ollama_client = ollama::Client::new(endpoint).expect("Failed to initialize Ollama client");

    let mut conversation_history: Vec<Message> = vec![];

    let agent = ollama_client
        .agent(&model_name)
        .preamble("
            You are an excellent CLI & Web agent.
            - Use `execute_bash_command` to check local system status or run commands.
            - Use `web_search` to find latest information, documentation, or generic knowledge from the internet.
            Choose the best tool based on the user's request.
        ")
        .additional_params(json!({
            "num_ctx": 64000,
            "num_predict": -1
        }))
        .tool(CliExecutionTool{seconds:Arc::clone(&shared_seconds),is_tool_running: Arc::clone(&is_tool_running),})
        .tool(DuckDuckGoSearchTool{seconds:Arc::clone(&shared_seconds),is_tool_running: Arc::clone(&is_tool_running),})
        .tool(FetchWebPageTool{seconds:Arc::clone(&shared_seconds),is_tool_running: Arc::clone(&is_tool_running),})
        .default_max_turns(10)
        .build();

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
        let timer_seconds = Arc::clone(&shared_seconds);
        let timer_tool_running = Arc::clone(&is_tool_running);
        let timer_handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(1));

            loop {
                interval.tick().await;
                if timer_tool_running.load(Ordering::SeqCst) == false {
                    let secs = timer_seconds.fetch_add(1, Ordering::SeqCst) + 1;
                    print!("\rAgent is thinking... ({}s)", secs);
                    let _ = io::stdout().flush();
                }
            }
        });

        print!("\rAgent is thinking... (0s)");
        io::stdout().flush().expect("Failed to flush");

        let mut full_response = String::new();
        let mut stream = agent
            .stream_chat(user_prompt, conversation_history.clone())
            .await;

        let mut is_first_chunk = true;
        while let Some(chunk) = stream.next().await {
            if let Ok(item) = chunk {
                if let Ok(json) = serde_json::to_value(&item) {
                    if json.get("type").and_then(|v| v.as_str()) == Some("streamAssistantItem") {
                        if let Some(text) = json.get("text").and_then(|v| v.as_str()) {
                            if is_first_chunk {
                                timer_handle.abort();
                                println!("\n--- AI Response ---");
                                is_first_chunk = false;
                            }
                            print!("{}", text);

                            use std::io::{self, Write};
                            let _ = io::stdout().flush();

                            full_response.push_str(text);
                        }
                    }
                }
            }
        }
        if is_first_chunk {
            timer_handle.abort();
        }
        conversation_history.push(rig::completion::Message::user(user_prompt.to_string()));
        conversation_history.push(rig::completion::Message::assistant(full_response));
    }
}
