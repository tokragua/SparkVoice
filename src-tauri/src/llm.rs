use anyhow::{anyhow, Result};
use reqwest::blocking::Client;
use serde_json::{json, Value};
use std::time::Duration;

use crate::settings::AppSettings;

pub fn extract_knowledge(settings: &AppSettings, text: &str) -> Result<Value> {
    if text.trim().is_empty() {
        return Ok(json!([]));
    }

    let model = &settings.llm_model;

    log::info!("Using LLM Model for Graph Extraction: {}", model);

    let system_prompt = "You are an Insight & Knowledge Extraction Agent. Your task is to map out ideas, topics, and connections from transcriptions.

Be eager to find connections. Extract ANY meaningful entities (people, projects, software, tasks, states of mind, locations, or general concepts/topics).
Determine how they relate. Even if a thought is general, represent it as a (Source Entity, Relation, Target Entity) triplet.
Relationships can be simple (e.g., 'is', 'related to', 'wants', 'working on', 'discussed', 'feeling').

Output valid JSON in this EXACT format:
{
  \"triplets\": [
    {\"source\": \"Entity A\", \"relation\": \"RELATED_TO\", \"target\": \"Topic B\", \"context\": \"brief explanation\"}
  ]
}

Keep entity names clear and concise.
CRITICAL: DO NOT REPEAT identical triplets. Each relationship should be unique.
Output ONLY the JSON object. Do not include markdown formatting like ```json. If no relations are found, return {\"triplets\": []}.";

    let client = Client::builder()
        .timeout(Duration::from_secs(600)) // Extraction can take a bit, especially for large logs or complex relationships
        .build()?;

    let request_body = json!({
        "model": model,
        "prompt": text,
        "system": system_prompt,
        "stream": false,
        "format": "json", // Force Ollama to output valid JSON
        "options": {
            "num_predict": 4096,
            "temperature": 0.2, // Very low temperature for high stability
            "repeat_penalty": 1.1, // Discourage repetition loops
            "repeat_last_n": 128   // Check last 128 tokens for repetition
        }
    });

    log::info!("Sending transcription to LLM API ({} chars)...", text.len());
    let start_time = std::time::Instant::now();

    let res = client
        .post(&settings.llm_api_url)
        .json(&request_body)
        .send()?;

    if !res.status().is_success() {
        let err_text = res.text().unwrap_or_default();
        return Err(anyhow!("LLM request failed: {}", err_text));
    }

    let response_json: Value = res.json()?;
    let duration = start_time.elapsed();
    
    // Log performance metrics
    let prompt_tokens = response_json.get("prompt_eval_count").and_then(|v| v.as_u64()).unwrap_or(0);
    let gen_tokens = response_json.get("eval_count").and_then(|v| v.as_u64()).unwrap_or(0);
    log::info!("LLM Extraction finished in {:?}. Tokens: {} prompt, {} gen", duration, prompt_tokens, gen_tokens);

    // Ollama returns the generated text in `response` field
    if let Some(generated_text) = response_json.get("response").and_then(|v: &Value| v.as_str()) {
        // Parse the generated text as JSON
        match serde_json::from_str::<Value>(generated_text) {
            Ok(json_data) => {
                if json_data.is_array() {
                    Ok(json_data)
                } else if json_data.is_object() {
                    // Check if it's the new wrapped format {"triplets": [...]}
                    if let Some(triplets) = json_data.get("triplets") {
                        if triplets.is_array() {
                            return Ok(triplets.clone());
                        }
                    }
                    // Handle case where LLM returns {} or another object
                    Ok(serde_json::json!([]))
                } else {
                    Err(anyhow!("LLM returned valid JSON but not an object/array: {}", generated_text))
                }
            }
            Err(e) => {
                Err(anyhow!("LLM output could not be parsed as JSON: {} (Output: {})", e, generated_text))
            }
        }
    } else {
        Err(anyhow!("Unexpected response format from Ollama"))
    }
}
