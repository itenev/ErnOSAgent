use super::*;

#[test]
fn test_review_card_creation() {
    let card = ReviewCard::new("course1", "lesson1", 0, "What is 2+2?", "4");
    assert_eq!(card.box_level, 0);
    assert_eq!(card.consecutive_correct, 0);
    assert_eq!(card.consecutive_wrong, 0);
    assert_eq!(card.total_reviews, 0);
    assert_eq!(card.question, "What is 2+2?");
    assert_eq!(card.expected_answer, "4");
}

#[test]
fn test_leitner_intervals() {
    assert_eq!(LEITNER_INTERVALS_DAYS[0], 1);  // Box 1: daily
    assert_eq!(LEITNER_INTERVALS_DAYS[1], 3);  // Box 2: 3 days
    assert_eq!(LEITNER_INTERVALS_DAYS[2], 7);  // Box 3: weekly
    assert_eq!(LEITNER_INTERVALS_DAYS[3], 14); // Box 4: biweekly
    assert_eq!(LEITNER_INTERVALS_DAYS[4], 30); // Box 5: monthly
}

#[test]
fn test_due_cards_empty() {
    let deck = ReviewDeck::new();
    assert!(deck.due_cards(Utc::now()).is_empty());
}

#[test]
fn test_due_cards_overdue() {
    let mut deck = ReviewDeck::new();
    let mut card = ReviewCard::new("c1", "l1", 0, "Q?", "A");
    card.next_review = Utc::now() - Duration::hours(1); // Overdue
    deck.add_card(card).unwrap();
    assert_eq!(deck.due_cards(Utc::now()).len(), 1);
}

#[test]
fn test_due_cards_future() {
    let mut deck = ReviewDeck::new();
    let mut card = ReviewCard::new("c1", "l1", 0, "Q?", "A");
    card.next_review = Utc::now() + Duration::days(30); // Far future
    deck.add_card(card).unwrap();
    assert_eq!(deck.due_cards(Utc::now()).len(), 0);
}

#[test]
fn test_record_correct_promotes() {
    let mut deck = ReviewDeck::new();
    let card = ReviewCard::new("c1", "l1", 0, "Q?", "A");
    let id = card.id.clone();
    deck.add_card(card).unwrap();

    deck.record_result(&id, true).unwrap();
    let c = deck.cards.iter().find(|c| c.id == id).unwrap();
    assert_eq!(c.box_level, 1); // 0 → 1
    assert_eq!(c.consecutive_correct, 1);
    assert_eq!(c.total_reviews, 1);

    deck.record_result(&id, true).unwrap();
    let c = deck.cards.iter().find(|c| c.id == id).unwrap();
    assert_eq!(c.box_level, 2); // 1 → 2
}

#[test]
fn test_record_correct_max_box() {
    let mut deck = ReviewDeck::new();
    let mut card = ReviewCard::new("c1", "l1", 0, "Q?", "A");
    card.box_level = MAX_BOX_LEVEL;
    let id = card.id.clone();
    deck.add_card(card).unwrap();

    deck.record_result(&id, true).unwrap();
    let c = deck.cards.iter().find(|c| c.id == id).unwrap();
    assert_eq!(c.box_level, MAX_BOX_LEVEL); // Stays at max
}

#[test]
fn test_record_wrong_demotes() {
    let mut deck = ReviewDeck::new();
    let mut card = ReviewCard::new("c1", "l1", 0, "Q?", "A");
    card.box_level = 3; // Box 4
    let id = card.id.clone();
    deck.add_card(card).unwrap();

    deck.record_result(&id, false).unwrap();
    let c = deck.cards.iter().find(|c| c.id == id).unwrap();
    assert_eq!(c.box_level, 0); // Back to box 1
    assert_eq!(c.consecutive_wrong, 1);
    assert_eq!(c.consecutive_correct, 0);
}

#[test]
fn test_add_card_dedup() {
    let mut deck = ReviewDeck::new();
    let card1 = ReviewCard::new("c1", "l1", 0, "Q1", "A1");
    let card2 = ReviewCard::new("c1", "l1", 0, "Q1-dup", "A1-dup");
    deck.add_card(card1).unwrap();
    deck.add_card(card2).unwrap(); // Same course+lesson+scene → skipped
    assert_eq!(deck.count(), 1);
    assert_eq!(deck.cards[0].question, "Q1"); // Original kept
}

#[test]
fn test_add_card_different_scenes() {
    let mut deck = ReviewDeck::new();
    deck.add_card(ReviewCard::new("c1", "l1", 0, "Q1", "A1")).unwrap();
    deck.add_card(ReviewCard::new("c1", "l1", 1, "Q2", "A2")).unwrap();
    assert_eq!(deck.count(), 2);
}

#[test]
fn test_retention_stats_empty() {
    let deck = ReviewDeck::new();
    let stats = deck.retention_stats();
    assert_eq!(stats.total_cards, 0);
    assert_eq!(stats.cards_due, 0);
    assert_eq!(stats.avg_box_level, 0.0);
}

#[test]
fn test_retention_stats() {
    let mut deck = ReviewDeck::new();
    let mut c1 = ReviewCard::new("c1", "l1", 0, "Q1", "A1");
    c1.box_level = 2;
    c1.total_reviews = 5;
    c1.consecutive_correct = 3;
    deck.add_card(c1).unwrap();

    let mut c2 = ReviewCard::new("c1", "l1", 1, "Q2", "A2");
    c2.box_level = 4;
    c2.total_reviews = 10;
    c2.consecutive_correct = 7;
    deck.add_card(c2).unwrap();

    let stats = deck.retention_stats();
    assert_eq!(stats.total_cards, 2);
    assert!((stats.avg_box_level - 3.0).abs() < 0.01); // (2+4)/2
}

#[test]
fn test_persistence_roundtrip() {
    let tmp = tempfile::TempDir::new().unwrap();
    let path = tmp.path().join("deck.json");

    {
        let mut deck = ReviewDeck::open(&path).unwrap();
        deck.add_card(ReviewCard::new("c1", "l1", 0, "Q?", "A")).unwrap();
        deck.add_card(ReviewCard::new("c2", "l2", 0, "Q2?", "A2")).unwrap();
    }

    let deck2 = ReviewDeck::open(&path).unwrap();
    assert_eq!(deck2.count(), 2);
    assert_eq!(deck2.cards[0].question, "Q?");
}

#[test]
fn test_generate_review_cards_empty() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = CurriculumStore::open(tmp.path()).unwrap();
    let cards = generate_review_cards(&store);
    assert!(cards.is_empty());
}

#[test]
fn test_record_result_missing_card() {
    let mut deck = ReviewDeck::new();
    assert!(deck.record_result("nonexistent", true).is_err());
}

#[test]
fn test_cross_level_primary_no_review() {
    let deck = ReviewDeck::new();
    let cards = cross_level_cards(&deck, EducationLevel::Primary, 10);
    assert!(cards.is_empty());
}

#[test]
fn test_due_count() {
    let mut deck = ReviewDeck::new();
    let mut c1 = ReviewCard::new("c1", "l1", 0, "Q1", "A1");
    c1.next_review = Utc::now() - Duration::hours(1);
    deck.add_card(c1).unwrap();

    let mut c2 = ReviewCard::new("c1", "l1", 1, "Q2", "A2");
    c2.next_review = Utc::now() + Duration::days(30);
    deck.add_card(c2).unwrap();

    assert_eq!(deck.due_count(Utc::now()), 1);
    assert_eq!(deck.count(), 2);
}
