use super::fixtag::FixTag;
use super::order::{Order, Side};
use chrono::Utc;
use std::collections::HashMap;

// Struct representing a FIX message
#[derive(Debug, Clone, PartialEq)]
pub struct FixMessage {
    pub fields: HashMap<FixTag, String>,
}

impl FixMessage {
    // Function to create a new FIX message
    pub fn new() -> FixMessage {
        FixMessage {
            fields: HashMap::new(),
        }
    }

    pub fn extract_tag_value<'a>(message: &'a str, tag: &'a str) -> Option<&'a str> {
        let tag_start = format!("{}=", tag);
        let tag_end = '\u{1}';

        if let Some(tag_start_pos) = message.find(&tag_start) {
            let tag_value_start = tag_start_pos + tag_start.len();
            if let Some(tag_end_pos) = message[tag_value_start..].find(tag_end) {
                let tag_value_end = tag_value_start + tag_end_pos;
                return Some(&message[tag_value_start..tag_value_end]);
            }
        }

        None
    }

    pub fn add_field(&mut self, tag: FixTag, value: &str) {
        self.fields.insert(tag, value.to_string());
    }

    pub fn remove_field(&mut self, tag: &FixTag) {
        self.fields.remove(tag);
    }

    pub fn encode(&mut self) -> String {
        self.add_field(FixTag::SendingTime, &Self::get_time());
        let mut sorted_fields: Vec<(&FixTag, &String)> = self.fields.iter().collect();
        sorted_fields.sort_by_key(|(tag, _)| *tag);

        let encoded_message: String = sorted_fields
            .iter()
            .map(|(tag, value)| format!("{}={}|", tag, value))
            .collect();
        format!("{}{}", encoded_message, "\x01")
    }

    pub fn decode(message: &str, delimiters: &str) -> FixMessage {
        let mut fields: HashMap<FixTag, String> = HashMap::new();

        let tags_values: Vec<&str> = message
            .trim_end_matches(delimiters)
            .split(delimiters)
            .collect();

        for tag_value in tags_values {
            let tag_value_split: Vec<&str> = tag_value.split('=').collect();
            if tag_value_split.len() == 2 {
                fields.insert(
                    match tag_value_split[0].parse::<FixTag>().ok() {
                        Some(tag) => tag,
                        None => {
                            println!(
                                "[MESSAGE] Tag {} is not a valid FIX tag, skipping",
                                tag_value_split[0]
                            );
                            continue;
                        }
                    },
                    tag_value_split[1].to_string(),
                );
            }
        }
        FixMessage { fields }
    }

    pub fn get_time() -> String {
        let now = Utc::now();
        now.format("%Y%m%d-%H:%M:%S%.3f").to_string()
    }

    pub fn to_order(&self) -> Option<Order> {
        let symbol = self.fields.get(&FixTag::Symbol)?;
        let quantity = self.fields.get(&FixTag::OrderQty)?.parse::<u32>().ok()?;
        let price = self.fields.get(&FixTag::Price)?.parse::<f64>().ok()?;
        let side = match self.fields.get(&FixTag::Side)?.parse::<isize>().ok()? {
            1 => Side::Buy,
            2 => Side::Sell,
            _ => return None,
        };

        Some(Order::new(symbol, quantity, price, side))
    }
}

#[test]
fn test_new_fix_message() {
    let fix_message = FixMessage::new();
    assert!(fix_message.fields.is_empty());
}

#[test]
fn test_add_field() {
    let mut fix_message = FixMessage::new();
    fix_message.add_field(FixTag::BeginString, "FIX.4.2");
    assert_eq!(fix_message.fields.len(), 1);
    assert_eq!(
        fix_message.fields.get(&FixTag::BeginString).unwrap(),
        "FIX.4.2"
    );
}

#[test]
fn test_encode_fix_message() {
    let mut fix_message = FixMessage::new();
    fix_message.add_field(FixTag::BeginString, "FIX.4.2");
    fix_message.add_field(FixTag::MsgType, "A");
    fix_message.add_field(FixTag::SenderCompID, "SENDER");
    fix_message.add_field(FixTag::TargetCompID, "TARGET");
    assert_eq!(
        fix_message.encode(),
        format!(
            "8=FIX.4.2|35=A|49=SENDER|56=TARGET|52={}|\x01",
            fix_message.fields.get(&FixTag::SendingTime).unwrap()
        )
    );
}

#[test]
fn test_decode_fix_message() {
    let fix_message = FixMessage::decode(&format!("8=FIX.4.2|35=A|49=SENDER|56=TARGET|\x01"), "|");
    assert_eq!(fix_message.fields.len(), 4);
    assert_eq!(
        fix_message.fields.get(&FixTag::BeginString).unwrap(),
        "FIX.4.2"
    );
    assert_eq!(fix_message.fields.get(&FixTag::MsgType).unwrap(), "A");
    assert_eq!(
        fix_message.fields.get(&FixTag::SenderCompID).unwrap(),
        "SENDER"
    );
    assert_eq!(
        fix_message.fields.get(&FixTag::TargetCompID).unwrap(),
        "TARGET"
    );
}

#[test]
fn test_get_time() {
    let now = Utc::now();
    assert_eq!(
        FixMessage::get_time(),
        now.format("%Y%m%d-%H:%M:%S%.3f").to_string()
    );
}

#[test]
fn test_to_order() {
    let mut fix_message = FixMessage::new();
    fix_message.add_field(FixTag::Symbol, "AAPL");
    fix_message.add_field(FixTag::OrderQty, "100");
    fix_message.add_field(FixTag::Price, "100.00");
    fix_message.add_field(FixTag::Side, "1");
    let order = fix_message.to_order().unwrap();
    assert_eq!(order.symbol, "AAPL");
    assert_eq!(order.quantity, 100);
    assert_eq!(order.price, 100.00);
    assert_eq!(order.side, Side::Buy);
}
