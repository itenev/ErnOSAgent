use super::*;

#[test]
fn test_mlx_config_primary() {
    let cfg = MlxTrainConfig::for_level(
        EducationLevel::Primary,
        Path::new("models/test"),
        Path::new("data"),
    );
    assert!((cfg.learning_rate - 2e-5).abs() < 1e-10);
    assert!((cfg.ewc_lambda - 0.0).abs() < f32::EPSILON);
    assert_eq!(cfg.epochs, 1);
}

#[test]
fn test_mlx_config_doctoral() {
    let cfg = MlxTrainConfig::for_level(
        EducationLevel::Doctoral,
        Path::new("models/test"),
        Path::new("data"),
    );
    assert!((cfg.learning_rate - 1e-6).abs() < 1e-10);
    assert!((cfg.ewc_lambda - 2.0).abs() < f32::EPSILON);
    assert!(cfg.output_dir.to_string_lossy().contains("doctoral"));
}

#[test]
fn test_mlx_config_ewc_scaling() {
    let levels = [
        (EducationLevel::Primary, 0.0),
        (EducationLevel::Secondary, 0.1),
        (EducationLevel::Undergraduate, 0.5),
        (EducationLevel::Masters, 1.0),
        (EducationLevel::Doctoral, 2.0),
    ];
    for (level, expected_ewc) in levels {
        let cfg = MlxTrainConfig::for_level(level, Path::new("m"), Path::new("d"));
        assert!(
            (cfg.ewc_lambda - expected_ewc).abs() < f32::EPSILON,
            "{:?}: expected ewc={}, got={}",
            level, expected_ewc, cfg.ewc_lambda
        );
    }
}

#[test]
fn test_prepare_training_data() {
    let tmp = tempfile::TempDir::new().unwrap();
    let samples = vec![
        TrainingSample {
            id: "s1".into(),
            input: "What is Rust?".into(),
            output: "A systems programming language.".into(),
            method: crate::learning::TrainingMethod::Sft,
            quality_score: 0.9,
            timestamp: chrono::Utc::now(),
        },
        TrainingSample {
            id: "s2".into(),
            input: "What is Python?".into(),
            output: "A high-level interpreted language.".into(),
            method: crate::learning::TrainingMethod::Sft,
            quality_score: 0.85,
            timestamp: chrono::Utc::now(),
        },
    ];
    let path = prepare_training_data(&samples, tmp.path()).unwrap();
    assert!(path.exists());

    // Verify JSONL format
    let content = std::fs::read_to_string(&path).unwrap();
    let lines: Vec<&str> = content.trim().lines().collect();
    assert_eq!(lines.len(), 2);

    // Verify each line is valid JSON with correct structure
    for line in &lines {
        let parsed: serde_json::Value = serde_json::from_str(line).unwrap();
        let msgs = parsed["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0]["role"], "user");
        assert_eq!(msgs[1]["role"], "assistant");
    }
}

#[test]
fn test_prepare_training_data_empty() {
    let tmp = tempfile::TempDir::new().unwrap();
    let result = prepare_training_data(&[], tmp.path());
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("empty samples"));
}

#[test]
fn test_parse_mlx_loss_standard() {
    let stdout = "Iter 1: Train loss 0.5432, ...\nIter 50: Train loss 0.2100, ...\nIter 100: Train loss 0.0987, ...";
    let loss = parse_mlx_loss(stdout);
    assert!((loss - 0.0987).abs() < 0.001);
}

#[test]
fn test_parse_mlx_loss_empty() {
    assert_eq!(parse_mlx_loss(""), 0.0);
}

#[test]
fn test_parse_mlx_loss_no_match() {
    let stdout = "Loading model...\nDone.";
    assert_eq!(parse_mlx_loss(stdout), 0.0);
}

#[test]
fn test_snapshot_ewc_missing_adapter() {
    let tmp = tempfile::TempDir::new().unwrap();
    let result = snapshot_ewc_params(
        &tmp.path().join("nonexistent"),
        "math",
        &tmp.path().join("ewc"),
    );
    assert!(result.is_err());
}

#[test]
fn test_snapshot_ewc_success() {
    let tmp = tempfile::TempDir::new().unwrap();
    let adapter_dir = tmp.path().join("adapters");
    std::fs::create_dir_all(&adapter_dir).unwrap();
    std::fs::write(adapter_dir.join("adapters.safetensors"), b"fake weights").unwrap();

    let ewc_dir = tmp.path().join("ewc");
    let snapshot = snapshot_ewc_params(&adapter_dir, "linear algebra", &ewc_dir).unwrap();
    assert!(snapshot.exists());
    assert!(snapshot.to_string_lossy().contains("linear_algebra_snapshot"));
}
