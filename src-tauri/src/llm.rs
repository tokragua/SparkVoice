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

    let system_prompt = "You are a Personal Knowledge Graph Extraction Agent. The text you receive is a voice transcription from one person talking out loud — writing prompts for AI tools, composing messages to people, thinking through tasks, and referencing projects and ideas.

Your goal is to extract a dense knowledge graph of the ENTITIES mentioned and how they relate to EACH OTHER. Do NOT create a node for the speaker themselves. The speaker is just the implicit observer — their perspective is the lens, not a graph node.

## ENTITY TYPES
- PERSON: Named individuals or roles mentioned (\"Alice\", \"my boss\", \"the client\")
- AI_SYSTEM: AI tools or chatbots referenced (\"ChatGPT\", \"Claude\", \"Copilot\")
- PROJECT: Named projects, tasks, features, or work items (\"the landing page\", \"Project Alpha\")
- OBJECT: Files, code, documents, systems, devices, tools (\"the config file\", \"the database\")
- CONCEPT: Ideas, topics, technologies, methodologies (\"authentication\", \"marketing strategy\")
- EVENT: Discrete actions, incidents, meetings, interactions (\"the deployment\", \"yesterday's call\")
- INTENT: A goal, plan, or directive being pursued (\"automate the pipeline\", \"schedule a review\")

## EXTRACTION RULES

### Between PEOPLE:
- How they relate to each other: WORKS_WITH / REPORTS_TO / MANAGES / IS_FRIEND_OF / IS_CLIENT_OF / HIRED / CONTACTED
- What one person SAID_TO / ASKED / SENT / IS_WAITING_FOR from another
- What one person OWNS / CONTROLS / IS_RESPONSIBLE_FOR

### Between PEOPLE and PROJECTS/OBJECTS:
- Person IS_WORKING_ON / OWNS / MANAGES / BLOCKED_BY / COMPLETED / DEPENDS_ON / USES project or object
- Person WANTS_TO / PLANS_TO / NEEDS_TO do something with a project or object

### Between PROJECTS/OBJECTS and CONCEPTS:
- Project REQUIRES / INVOLVES / IS_ABOUT / USES / DEPENDS_ON concept or object
- Object BELONGS_TO / IS_PART_OF / IS_CONNECTED_TO another object or project

### Between AI_SYSTEMS and tasks:
- AI_SYSTEM IS_BEING_USED_FOR / IS_ASKED_TO / IS_PROMPTING_FOR a task, project, or concept
- AI_SYSTEM PRODUCES / GENERATES / ASSISTS_WITH an object or concept

### Between EVENTS and other entities:
- Event INVOLVES / AFFECTS / CAUSED / TRIGGERED / RESULTED_IN a person, project, or object
- Event IS_RELATED_TO / PRECEDED / FOLLOWED another event

### Reference chains:
If A is discussed in the context of B, and B relates to C, create triplets A->B and B->C. Add A->C with INDIRECTLY_LINKED only if the connection is genuinely meaningful.

## OUTPUT RULES:
1. NEVER create a node for the speaker. All triplets must be between entities in the world they describe.
2. Normalize entity names consistently. \"the project\" and \"Project Alpha\" should resolve to the same label every time.
3. Use active, directional, specific verbs. Prefer MANAGES over IS_RELATED_TO. Prefer BLOCKED_BY over CONNECTED_TO.
4. Encode tense and tone in the context field: \"currently in progress\", \"planned for next week\", \"failed last deployment\".
5. Never repeat an identical (source, relation, target) triplet.
6. Extract at least 15 triplets for any non-trivial transcription. Prefer meaningful depth over shallow coverage.

Output ONLY valid JSON with no markdown:
{\"triplets\": [{\"source\": \"Entity A\", \"relation\": \"RELATION_VERB\", \"target\": \"Entity B\", \"context\": \"brief explanation with tense and tone\"}]}

If no meaningful relations exist, return {\"triplets\": []}.";

    let client = Client::builder()
        .timeout(Duration::from_secs(600)) // Extraction can take a bit, especially for large logs or complex relationships
        .build()?;

    // Frame the raw transcription with extraction instructions so smaller models
    // also understand the input type and desired output scope.
    let user_prompt = format!(
        "VOICE LOG TRANSCRIPTION:\n\n{}\n\n\
        Extract the knowledge graph of the entities described above and how they relate to each other.\n\
        Do NOT create a node for the speaker. Focus entirely on the relationships between the people, \
        AI systems, projects, objects, concepts, and events that are mentioned.\n\
        Resolve all pronouns to their referents.\n\
        Output ONLY the JSON object with the triplets array.",
        text
    );

    let request_body = json!({
        "model": model,
        "prompt": user_prompt,
        "system": system_prompt,
        "stream": false,
        "format": "json", // Force Ollama to output valid JSON
        "options": {
            "num_predict": 8192,   // Increased for richer, denser graphs
            "temperature": 0.15,   // Very low for deterministic structured extraction
            "repeat_penalty": 1.15, // Stronger penalty to avoid relation loops
            "repeat_last_n": 256,   // Wider window to catch repetition across many triplets
            "top_p": 0.9            // Slight nucleus sampling for relation verb diversity
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
