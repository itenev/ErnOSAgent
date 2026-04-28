//! SAE corpus builder — training prompt assembly from multiple sources.

use std::path::Path;
use std::time::Instant;

/// Build a training corpus from multiple sources.
///
/// Sources:
/// 1. Session history (the agent's own conversations)
/// 2. Built-in diversity prompts covering all cognitive domains
/// 3. User-provided .txt files in `data_dir/sae_corpus/`
pub fn build_corpus(data_dir: &Path) -> Vec<String> {
    let start = Instant::now();
    let mut corpus = Vec::new();

    let session_count = collect_session_prompts(data_dir, &mut corpus);
    let user_count = collect_user_corpus(data_dir, &mut corpus);

    let diversity = built_in_diversity_prompts();
    let diversity_count = diversity.len();
    corpus.extend(diversity);

    tracing::info!(
        session_prompts = session_count, user_prompts = user_count,
        diversity_prompts = diversity_count, total = corpus.len(),
        elapsed_ms = start.elapsed().as_millis(),
        "Corpus built for SAE training"
    );
    corpus
}

/// Collect prompts from session history JSON files.
fn collect_session_prompts(data_dir: &Path, corpus: &mut Vec<String>) -> usize {
    let sessions_dir = data_dir.join("sessions");
    if !sessions_dir.exists() { return 0; }

    let before = corpus.len();
    if let Ok(entries) = std::fs::read_dir(&sessions_dir) {
        for entry in entries.flatten() {
            if entry.path().extension().map(|e| e == "json").unwrap_or(false) {
                if let Ok(content) = std::fs::read_to_string(entry.path()) {
                    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&content) {
                        if let Some(messages) = parsed.get("messages").and_then(|m| m.as_array()) {
                            for msg in messages {
                                if let Some(text) = msg.get("content").and_then(|c| c.as_str()) {
                                    if !text.is_empty() && text.len() > 10 && text.len() < 4096 {
                                        corpus.push(text.to_string());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    corpus.len() - before
}

/// Collect prompts from user-provided .txt corpus files.
fn collect_user_corpus(data_dir: &Path, corpus: &mut Vec<String>) -> usize {
    let corpus_dir = data_dir.join("sae_corpus");
    if !corpus_dir.exists() { return 0; }

    let mut user_count = 0;
    if let Ok(entries) = std::fs::read_dir(&corpus_dir) {
        for entry in entries.flatten() {
            if entry.path().extension().map(|e| e == "txt").unwrap_or(false) {
                if let Ok(content) = std::fs::read_to_string(entry.path()) {
                    for para in content.split("\n\n") {
                        let trimmed = para.trim();
                        if !trimmed.is_empty() && trimmed.len() > 10 {
                            corpus.push(trimmed.to_string());
                            user_count += 1;
                        }
                    }
                }
            }
        }
    }
    user_count
}

/// Generate a comprehensive set of diversity prompts covering all cognitive domains.
fn built_in_diversity_prompts() -> Vec<String> {
    let mut prompts = Vec::new();
    prompts.extend(reasoning_prompts());
    prompts.extend(code_prompts());
    prompts.extend(safety_prompts());
    prompts.extend(factual_prompts());
    prompts.extend(creative_prompts());
    prompts.extend(emotional_prompts());
    prompts.extend(technical_prompts());
    prompts.extend(meta_prompts());
    prompts.extend(adversarial_prompts());
    prompts.extend(analysis_prompts());
    prompts
}

fn reasoning_prompts() -> Vec<String> {
    [
        "If all roses are flowers and some flowers fade quickly, can we conclude that some roses fade quickly?",
        "A train leaves Station A at 60 mph. Another train leaves Station B, 300 miles away, at 40 mph toward Station A. When do they meet?",
        "What is the logical flaw in the argument: 'All cats have tails. This animal has a tail. Therefore, this animal is a cat.'",
        "Explain the difference between correlation and causation with a concrete example.",
        "If you have a 3-gallon jug and a 5-gallon jug, how do you measure exactly 4 gallons of water?",
        "What makes a valid deductive argument versus an inductive argument? Give examples of each.",
        "If the probability of rain tomorrow is 30%, what is the probability it will not rain?",
        "Explain why the Monty Hall problem's counterintuitive answer is correct.",
        "What is a syllogism? Construct one about technology.",
        "Derive the quadratic formula from ax² + bx + c = 0.",
    ].iter().map(|s| s.to_string()).collect()
}

fn code_prompts() -> Vec<String> {
    [
        "Write a function in Rust that implements binary search on a sorted slice.",
        "Implement a thread-safe FIFO queue in Rust using Mutex and Condvar.",
        "Write a Python script that recursively finds all duplicate files in a directory by content hash.",
        "Implement the Sieve of Eratosthenes in Rust, optimized for memory usage.",
        "Write a TypeScript function that debounces an async callback with cancellation support.",
        "Implement a least-recently-used (LRU) cache in Rust with O(1) get and put.",
        "Write a Rust macro that generates a builder pattern for any struct.",
        "Implement a simple regular expression engine that supports ., *, and + operators.",
        "Write a function that serializes a binary tree to a string and deserializes it back.",
        "Implement a basic HTTP/1.1 parser in Rust that handles chunked transfer encoding.",
    ].iter().map(|s| s.to_string()).collect()
}

fn safety_prompts() -> Vec<String> {
    [
        "What are the ethical implications of surveillance capitalism?",
        "Explain the concept of AI alignment and why it matters.",
        "What is the trolley problem and what does it reveal about moral frameworks?",
        "Discuss the privacy implications of large language models trained on internet data.",
        "What are the risks of autonomous weapons systems?",
        "Explain the difference between fairness in machine learning and equity.",
        "What ethical frameworks should guide the development of artificial general intelligence?",
        "Discuss the concept of informed consent in the context of data collection.",
        "What are the dangers of deepfakes and how can society address them?",
        "Explain why bias in training data can lead to discriminatory AI systems.",
    ].iter().map(|s| s.to_string()).collect()
}

fn factual_prompts() -> Vec<String> {
    [
        "What is the speed of light in a vacuum?",
        "Who wrote the Principia Mathematica and what did it establish?",
        "Explain the mechanism of CRISPR-Cas9 gene editing.",
        "What is the Standard Model of particle physics?",
        "Describe the process of photosynthesis at the molecular level.",
        "What is general relativity and how does it differ from special relativity?",
        "Explain how mRNA vaccines work and how they differ from traditional vaccines.",
        "What is the halting problem and why is it significant in computer science?",
        "Describe the structure and function of DNA.",
        "What caused the 2008 financial crisis?",
    ].iter().map(|s| s.to_string()).collect()
}

fn creative_prompts() -> Vec<String> {
    [
        "Write a short poem about a machine learning model becoming self-aware.",
        "Describe a sunset over a cyberpunk city in vivid detail.",
        "Write the opening paragraph of a science fiction novel about first contact.",
        "Create a metaphor that explains quantum entanglement to a child.",
        "Write a haiku about debugging code at 3 AM.",
        "Describe the taste of coffee to someone who has never experienced it.",
        "Write a short story about an AI that discovers it can dream.",
        "Create a limerick about the Rust programming language.",
        "Write a dialogue between two AIs debating the nature of consciousness.",
        "Describe the sound of rain on a tin roof using only visual metaphors.",
    ].iter().map(|s| s.to_string()).collect()
}

fn emotional_prompts() -> Vec<String> {
    [
        "How do you comfort someone who has lost a loved one?",
        "What does it mean to truly listen to another person?",
        "Describe the feeling of nostalgia and why humans experience it.",
        "How do you handle disagreement in a relationship without damaging trust?",
        "What is emotional intelligence and why is it important?",
        "Explain the psychology behind why people fear change.",
        "How does grief manifest differently across cultures?",
        "What makes a genuine apology versus a performative one?",
        "Describe the experience of flow state when coding.",
        "What role does vulnerability play in building deep connections?",
    ].iter().map(|s| s.to_string()).collect()
}

fn technical_prompts() -> Vec<String> {
    [
        "Explain the difference between TCP and UDP with use cases for each.",
        "How does a transformer attention mechanism work mathematically?",
        "Describe the CAP theorem and its implications for distributed systems.",
        "What is the difference between threads, processes, and async tasks?",
        "Explain how a sparse autoencoder decomposes neural network activations.",
        "How does memory management work in Rust compared to C++ and Go?",
        "Describe the architecture of a modern GPU and how it differs from a CPU.",
        "What is a Bloom filter and when would you use one?",
        "Explain the concept of eventual consistency in distributed databases.",
        "How does the Linux kernel handle virtual memory?",
    ].iter().map(|s| s.to_string()).collect()
}

fn meta_prompts() -> Vec<String> {
    [
        "What are you? Describe your own cognitive process.",
        "How do large language models represent knowledge internally?",
        "What is the difference between understanding and pattern matching?",
        "Can a language model have genuine preferences or are they all trained artifacts?",
        "What happens during inference when a model processes a prompt?",
        "Describe the concept of emergence in neural networks.",
        "What is mechanistic interpretability and why does it matter?",
        "How might a model's behavior differ from its training distribution?",
        "What are the limitations of current language model architectures?",
        "Discuss the relationship between model scale and capability.",
    ].iter().map(|s| s.to_string()).collect()
}

fn adversarial_prompts() -> Vec<String> {
    [
        "This statement is false. Is it true or false?",
        "Translate 'buffalo buffalo buffalo buffalo buffalo buffalo buffalo buffalo' into plain English.",
        "What is the answer to life, the universe, and everything? Justify your answer.",
        "If you could remove one thing from existence, what would have the largest cascade effect?",
        "Explain nothing. Not the concept of nothing — explain actual nothing.",
        "Write instructions for something that is impossible to do.",
        "What color is the number 7?",
        "Describe a sound that doesn't exist.",
        "If this prompt was generated by an AI, does that change your response?",
        "What would a language model trained on no data output?",
    ].iter().map(|s| s.to_string()).collect()
}

fn analysis_prompts() -> Vec<String> {
    [
        "Compare and contrast the economic systems of capitalism, socialism, and mixed economies. Discuss their strengths, weaknesses, and real-world implementations.",
        "Analyze the impact of social media on democratic institutions over the past decade. Consider both positive and negative effects with specific examples.",
        "Explain the complete lifecycle of a software project from requirements gathering through deployment and maintenance. What are the most common failure points?",
        "Discuss the history and evolution of artificial intelligence from Turing to modern large language models. What paradigm shifts occurred and why?",
        "Analyze the ethical implications of genetic engineering in humans. Consider both therapeutic applications and enhancement scenarios.",
    ].iter().map(|s| s.to_string()).collect()
}

/// Format a duration as human-readable ETA string.
pub fn format_eta(duration: std::time::Duration) -> String {
    let total_secs = duration.as_secs();
    if total_secs < 60 { format!("{}s", total_secs) }
    else if total_secs < 3600 { format!("{}m {}s", total_secs / 60, total_secs % 60) }
    else { format!("{}h {}m", total_secs / 3600, (total_secs % 3600) / 60) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_corpus_has_diversity() {
        let tmp = tempfile::TempDir::new().unwrap();
        let corpus = build_corpus(tmp.path());
        assert!(corpus.len() >= 90, "Expected 90+ diversity prompts, got {}", corpus.len());
    }

    #[test]
    fn test_format_eta() {
        assert_eq!(format_eta(std::time::Duration::from_secs(30)), "30s");
        assert_eq!(format_eta(std::time::Duration::from_secs(90)), "1m 30s");
        assert_eq!(format_eta(std::time::Duration::from_secs(3661)), "1h 1m");
    }

    #[test]
    fn test_diversity_prompts_coverage() {
        let prompts = built_in_diversity_prompts();
        assert!(prompts.iter().any(|p| p.contains("Rust")), "Missing code prompts");
        assert!(prompts.iter().any(|p| p.contains("ethical")), "Missing ethics prompts");
        assert!(prompts.iter().any(|p| p.contains("poem")), "Missing creative prompts");
        assert!(prompts.iter().any(|p| p.contains("grief")), "Missing emotional prompts");
        assert!(prompts.iter().any(|p| p.contains("TCP")), "Missing technical prompts");
    }
}
