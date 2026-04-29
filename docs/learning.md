# Learning Pipeline

The learning subsystem provides on-device incremental training. All code lives in `src/learning/`. The pipeline is **fully integrated** — Observer verdicts automatically fill training buffers, and a background scheduler triggers sleep cycles when thresholds are met.

## Module Map

| File | Purpose |
|------|---------|
| `mod.rs` | `TrainingSample`, `TrainingMethod`, `PipelineStatus` structs |
| `teacher.rs` | Training orchestrator — SFT and preference training |
| `sleep.rs` | Background sleep consolidation cycle |
| `buffers.rs` | `GoldenBuffer` — high-quality sample storage |
| `buffers_rejection.rs` | `RejectionBuffer` + `PreferencePair` for DPO/KTO |
| `observer_buffer.rs` | Observer-scored sample buffer |
| `manifest.rs` | Training run metadata tracking |
| `distill.rs` | Knowledge distillation utilities |
| `curriculum.rs` | Course/lesson/scene data model, progress tracking, JSON persistence |
| `verification.rs` | Ground truth verification gate + `QuarantineBuffer` |
| `student.rs` | Student session loop — scene processing, local buffer accumulation |
| `research.rs` | PhD research engine — arXiv ingestion, hypothesis generation, adversarial defense |
| `mlx_bridge.rs` | MLX subprocess LoRA fine-tuning bridge — JSONL prep, EWC snapshots |
| `graduation.rs` | Education level promotion — gate criteria, adapter validation, model fusion |
| `review.rs` | Spaced repetition — Leitner box review cards, retention stats |

### LoRA Subsystem (`src/learning/lora/`)

| File | Purpose |
|------|---------|
| `mod.rs` | `LoraConfig` struct |
| `weights.rs` | `LoraLayer` — A/B matrix storage with forward pass |
| `forward.rs` | Forward pass through LoRA adapter |
| `training.rs` | `train_step()` and `train_epoch()` — finite-difference gradient computation |
| `loss.rs` | Cross-entropy loss |
| `loss_dpo.rs` | Direct Preference Optimization loss |
| `loss_kto.rs` | Kahneman-Tversky Optimization loss |
| `loss_simpo.rs` | Simple Preference Optimization loss |
| `optimizer.rs` | SGD optimizer |
| `adapters.rs` | Multi-adapter management |
| `ewc.rs` | Elastic Weight Consolidation (catastrophic forgetting prevention) |
| `training_alignment.rs` | Alignment training utilities |

### GRPO Subsystem (`src/learning/grpo/`)

| File | Purpose |
|------|---------|
| `mod.rs` | Module declarations |
| `generation.rs` | Candidate response generation (online + offline) |
| `rewards.rs` | Multi-signal reward scoring (length, relevance, structure) + advantage computation |
| `training.rs` | GRPO loss with KL penalty |

## Core Types

### TrainingSample

```rust
pub struct TrainingSample {
    pub id: String,
    pub input: String,
    pub output: String,
    pub method: TrainingMethod,
    pub quality_score: f32,
    pub timestamp: DateTime<Utc>,
}
```

### TrainingMethod

```rust
pub enum TrainingMethod { Sft, Orpo, SimPO, Kto, Dpo, Grpo }
```

### LoraConfig

```rust
pub struct LoraConfig {
    pub rank: usize,          // default: 4
    pub alpha: f32,           // default: 8.0
    pub learning_rate: f64,   // default: 1e-4
    pub model_dim: usize,     // default: 3584
}
```

### LoraLayer

```rust
pub struct LoraLayer {
    pub name: String,
    pub a_matrix: Vec<Vec<f32>>,  // [rank × input_dim]
    pub b_matrix: Vec<Vec<f32>>,  // [output_dim × rank]
    pub rank: usize,
    pub alpha: f32,
    pub input_dim: usize,
    pub output_dim: usize,
}
```

## Training Flow

### SFT (Supervised Fine-Tuning)

```
Teacher.train_sft(samples) →
  for each sample:
    input_vec = text_to_vec(sample.input)
    target_vec = text_to_vec(sample.output)
    loss = training::train_step(&mut lora, &input, &target, lr)
  → TrainingResult { method: "SFT", samples, loss }
```

### Preference Training (DPO)

```
Teacher.train_preference(pairs, method) →
  for each PreferencePair:
    chosen_vec → LoRA forward
    rejected_vec → LoRA forward
    loss = chosen_loss - rejected_loss margin
  → TrainingResult { method: "DPO", samples, loss }
```

### GRPO (Group Relative Policy Optimization)

```
1. generate_candidates_offline(prompt, n) → n candidate responses
2. score_group(candidates, query) → reward scores (length + relevance + structure)
3. compute_advantages(scores) → zero-mean advantages
4. train_step(candidates, query, kl_coeff) → policy loss with KL penalty
```

**Reward signals** in `rewards::score_group()`:
- **Length score**: `min(len / 200.0, 1.5)` — rewards substantive responses
- **Relevance score**: Keyword overlap between response and query
- **Structure score**: Presence of paragraphs, lists, code blocks

### Gradient Computation

LoRA training uses **finite-difference gradient estimation** (not autograd). In `training::train_step()`:

1. Compute forward loss at current B-matrix weights
2. For each weight, perturb by `epsilon` (1e-4), compute loss again
3. Gradient = `(perturbed_loss - base_loss) / epsilon`
4. Update: `weight -= learning_rate × gradient`

This approach requires no autograd library and works on any hardware.

## Sleep Consolidation

`sleep::run_sleep_cycle()` is a background task that:

1. Drains the `GoldenBuffer` (up to 32 samples)
2. Runs SFT training via `Teacher.train_sft()`
3. Drains the `RejectionBuffer` (up to 16 pairs)
4. Runs DPO training via `Teacher.train_preference()`
5. Calls `synaptic.decay_all(0.95)` to weaken inactive knowledge graph nodes
6. Calls `lessons.decay_unused(0.98, 0.3)` to prune stale lessons

The sleep cycle is triggered by the `sleep_cycle` job in the cron engine (`src/scheduler/`). The scheduler ticks every 15 seconds, checking all enabled jobs against their schedule. The `sleep_cycle` job runs every 5 minutes and triggers when buffer thresholds are met:
- GoldenBuffer ≥ 10 samples, OR
- RejectionBuffer ≥ 5 pairs

## GoldenBuffer

```rust
pub struct GoldenBuffer {
    samples: Vec<TrainingSample>,
    max_size: usize,
}
```

- `add(sample)` — append (drops oldest if at capacity)
- `drain_batch(count)` — remove and return up to `count` samples
- `count()` — current buffer size

## PreferencePair

```rust
pub struct PreferencePair {
    pub id: String,
    pub input: String,
    pub chosen: String,
    pub rejected: String,
    pub rejection_reason: String,
    pub timestamp: DateTime<Utc>,
}
```

Created when the Observer rejects a response — the original (rejected) and retry (chosen) form a preference pair for DPO training.

## Live Integration

### Observer → Training Buffers

`src/web/training_capture.rs` captures Observer verdicts as training signals:
- **Approved** responses → `capture_approved()` → `GoldenBuffer` (SFT)
- **Rejected** responses + retried replacement → `capture_rejection()` → `RejectionBuffer` (DPO pairs)

Both operate as fire-and-forget `tokio::spawn` background tasks.

### Learning Tool

The `learning` tool (`src/tools/learning_tool.rs`) provides live access to the training pipeline through `AppState`:
- `status` — real buffer counts and pipeline readiness
- `buffer_stats` — detailed capacity info
- `trigger_training` — queue a training run
- `sleep` — manually trigger the sleep consolidation cycle

### LoRA Adapter Loading

`LlamaCppConfig.lora_adapter` specifies a trained adapter to load at inference via `--lora` flag. After the scheduler completes a sleep cycle and saves an adapter, a server restart loads it into the model.

## Schooling Pipeline (K-12 → PhD)

The schooling pipeline enables Ern-OS to autonomously progress through a full education system. It uses 5 education levels with incremental weight updates at each stage.

### Education Levels

| Level | EWC Lambda | Pass Threshold | Learning Mode |
|-------|-----------|----------------|---------------|
| Primary | 0.1 | 0.5 | Read → Repeat → Quiz |
| Secondary | 0.3 | 0.6 | Read → Reason → Apply |
| Undergraduate | 0.5 | 0.65 | Read → Analyze → Synthesize |
| Masters | 0.7 | 0.75 | Read → Evaluate → Create |
| Doctoral | 0.9 | 0.8 | Survey → Hypothesize → Experiment → Defend |

### Curriculum Store (`curriculum.rs`)

Manages courses, lessons, scenes, and student progress. Persists to `data/curriculum/` as JSON.

- **Course** — title, level, subject, lessons, prerequisites, completion criteria, source
- **Lesson** — ordered scenes with objectives and prerequisites
- **Scene** — teaching unit with scene type (Quiz, Lecture, Exercise, Essay, etc.), interaction type, expected output, and difficulty
- **CourseProgress** — per-course completion tracking with quiz scores

Curriculum sources: OpenMAIC ZIP, OSSU GitHub, arXiv papers, custom JSONL.

### Verification Gate (`verification.rs`)

Every student answer passes through a verification stage before entering the Golden Buffer:

1. **Confirmed** — answer matches ground truth or external verification → Golden Buffer
2. **Contradicted** — answer proven wrong → Rejection Buffer
3. **Unverifiable** — cannot confirm/deny → Quarantine Buffer

This prevents hallucination feedback loops where the model trains on its own wrong answers.

### Student Loop (`student.rs`)

Processes scenes using `StudentSession` with local buffers:

1. Scene prompt generated per education level and interaction type
2. Student answer via `provider.chat_sync()`
3. Verification against expected output
4. Results accumulated in local buffers (no shared locks during inference)
5. Batch flush to shared state at lesson end (microsecond lock durations)

Preemption: if `cancel_flag` is set (user activity), saves position and returns.

### Research Engine (`research.rs`)

PhD-level learning — manages `ResearchProject` lifecycle:

1. **Literature Survey** — arXiv search via `web_search`, paper metadata extraction, embedding storage
2. **Hypothesis Generation** — novelty and testability scoring via LLM
3. **Experimentation** — handled by curriculum scenes
4. **Paper Writing** — handled by curriculum scenes
5. **Defense** — adversarial self-play (Model A attacks, Model B defends, both scored)

Gate criteria: survey needs ≥10 papers, hypothesis needs ≥3 scored, defense needs ≥3 rounds with avg quality >0.7.

### MLX Training Bridge (`mlx_bridge.rs`)

Subprocess-based LoRA fine-tuning using Apple's MLX framework:

- `prepare_training_data()` — converts `TrainingSample` → JSONL chat format
- `snapshot_ewc_params()` — saves current adapter weights as EWC θ* reference
- `run_mlx_lora()` — shells out to `python3 -m mlx_lm.lora` with level-scaled hyperparameters
- `fuse_adapter()` — `python3 -m mlx_lm.fuse` to merge adapter into base model
- `check_mlx_available()` — graceful feature gating (disabled if Python/MLX not installed)

Learning rate scales with education level: Primary 1e-5, Secondary 5e-6, Undergraduate 2e-6, Masters 1e-6, Doctoral 5e-7.

### Graduation Pipeline (`graduation.rs`)

Auto-promotes between education levels when gate criteria are met:

| Level | Required Courses | Min Score | Capstone |
|-------|-----------------|-----------|----------|
| Primary → Secondary | 3 | 0.60 | No |
| Secondary → Undergraduate | 4 | 0.65 | No |
| Undergraduate → Masters | 5 | 0.70 | No |
| Masters → Doctoral | 3 | 0.75 | Yes |
| Doctoral → Complete | 1 | 0.80 | Yes (defense) |

Graduation history persisted to `data/graduation_history.json`.

### Spaced Repetition (`review.rs`)

Leitner box system preventing curriculum-level catastrophic forgetting:

- **Box 1**: review daily (1 day)
- **Box 2**: review every 3 days
- **Box 3**: review weekly (7 days)
- **Box 4**: review biweekly (14 days)
- **Box 5**: review monthly (30 days)

Correct → promote one box. Wrong → demote to box 1 + rejection buffer for retraining.

Cards generated from completed course Quiz/Exercise/CriticalAnalysis scenes with expected outputs. Deck persisted to `data/review_deck.json`.

### Scheduler Jobs

Three learning jobs run via the cron engine (`src/scheduler/learning_tasks.rs`):

| Job | Interval | Default |
|-----|----------|---------|
| `attend_class` | 4 hours | Disabled |
| `conduct_research` | 24 hours | Disabled |
| `spaced_review` | 12 hours | Disabled |

All disabled until the user adds courses to the curriculum. Enable via the scheduler API or WebUI.

