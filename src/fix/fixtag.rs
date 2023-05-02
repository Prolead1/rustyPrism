use std::{fmt::Display, hash::Hash, str::FromStr};

#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub enum FixTag {
    BeginString,
    BodyLength,
    MsgType,
    SenderCompID,
    TargetCompID,
    MsgSeqNum,
    SendingTime,
    Symbol,
    Side,
    OrderQty,
    Price,
    OrdType,
    OrderID,
    ExecID,
    LeavesQty,
    CumQty,
    AvgPx,
    Text,
    CheckSum,
}

impl FixTag {
    fn tag_value(&self) -> u32 {
        match self {
            FixTag::BeginString => 8,
            FixTag::BodyLength => 9,
            FixTag::MsgType => 35,
            FixTag::SenderCompID => 49,
            FixTag::TargetCompID => 56,
            FixTag::MsgSeqNum => 34,
            FixTag::SendingTime => 52,
            FixTag::CheckSum => 10,
            FixTag::Symbol => 55,
            FixTag::Side => 54,
            FixTag::OrderQty => 38,
            FixTag::Price => 44,
            FixTag::OrdType => 40,
            FixTag::OrderID => 37,
            FixTag::ExecID => 17,
            FixTag::LeavesQty => 151,
            FixTag::CumQty => 14,
            FixTag::AvgPx => 6,
            FixTag::Text => 58,
        }
    }
}

impl FromStr for FixTag {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "8" => Ok(FixTag::BeginString),
            "9" => Ok(FixTag::BodyLength),
            "35" => Ok(FixTag::MsgType),
            "49" => Ok(FixTag::SenderCompID),
            "56" => Ok(FixTag::TargetCompID),
            "34" => Ok(FixTag::MsgSeqNum),
            "52" => Ok(FixTag::SendingTime),
            "10" => Ok(FixTag::CheckSum),
            "55" => Ok(FixTag::Symbol),
            "54" => Ok(FixTag::Side),
            "38" => Ok(FixTag::OrderQty),
            "44" => Ok(FixTag::Price),
            "40" => Ok(FixTag::OrdType),
            "37" => Ok(FixTag::OrderID),
            "17" => Ok(FixTag::ExecID),
            "151" => Ok(FixTag::LeavesQty),
            "14" => Ok(FixTag::CumQty),
            "6" => Ok(FixTag::AvgPx),
            "58" => Ok(FixTag::Text),
            _ => Err(()),
        }
    }
}

impl Display for FixTag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = self.tag_value();
        write!(f, "{}", s)
    }
}

#[test]
fn test_fix_tag_from_str() {
    assert_eq!("8".parse::<FixTag>().unwrap(), FixTag::BeginString);
    assert_eq!("9".parse::<FixTag>().unwrap(), FixTag::BodyLength);
    assert_eq!("35".parse::<FixTag>().unwrap(), FixTag::MsgType);
    assert_eq!("49".parse::<FixTag>().unwrap(), FixTag::SenderCompID);
    assert_eq!("56".parse::<FixTag>().unwrap(), FixTag::TargetCompID);
    assert_eq!("34".parse::<FixTag>().unwrap(), FixTag::MsgSeqNum);
    assert_eq!("52".parse::<FixTag>().unwrap(), FixTag::SendingTime);
    assert_eq!("10".parse::<FixTag>().unwrap(), FixTag::CheckSum);
    assert_eq!("55".parse::<FixTag>().unwrap(), FixTag::Symbol);
    assert_eq!("54".parse::<FixTag>().unwrap(), FixTag::Side);
    assert_eq!("38".parse::<FixTag>().unwrap(), FixTag::OrderQty);
    assert_eq!("44".parse::<FixTag>().unwrap(), FixTag::Price);
    assert_eq!("40".parse::<FixTag>().unwrap(), FixTag::OrdType);
    assert_eq!("37".parse::<FixTag>().unwrap(), FixTag::OrderID);
    assert_eq!("17".parse::<FixTag>().unwrap(), FixTag::ExecID);
    assert_eq!("151".parse::<FixTag>().unwrap(), FixTag::LeavesQty);
    assert_eq!("14".parse::<FixTag>().unwrap(), FixTag::CumQty);
    assert_eq!("6".parse::<FixTag>().unwrap(), FixTag::AvgPx);
    assert_eq!("58".parse::<FixTag>().unwrap(), FixTag::Text);
    assert_eq!("".parse::<FixTag>().is_err(), true);
}

#[test]
fn test_fix_tag_to_string() {
    assert_eq!(FixTag::BeginString.to_string(), "8");
    assert_eq!(FixTag::BodyLength.to_string(), "9");
    assert_eq!(FixTag::MsgType.to_string(), "35");
    assert_eq!(FixTag::SenderCompID.to_string(), "49");
    assert_eq!(FixTag::TargetCompID.to_string(), "56");
    assert_eq!(FixTag::MsgSeqNum.to_string(), "34");
    assert_eq!(FixTag::SendingTime.to_string(), "52");
    assert_eq!(FixTag::CheckSum.to_string(), "10");
    assert_eq!(FixTag::Symbol.to_string(), "55");
    assert_eq!(FixTag::Side.to_string(), "54");
    assert_eq!(FixTag::OrderQty.to_string(), "38");
    assert_eq!(FixTag::Price.to_string(), "44");
    assert_eq!(FixTag::OrdType.to_string(), "40");
    assert_eq!(FixTag::OrderID.to_string(), "37");
    assert_eq!(FixTag::ExecID.to_string(), "17");
    assert_eq!(FixTag::LeavesQty.to_string(), "151");
    assert_eq!(FixTag::CumQty.to_string(), "14");
    assert_eq!(FixTag::AvgPx.to_string(), "6");
    assert_eq!(FixTag::Text.to_string(), "58");
}

#[test]
fn test_fix_tag_cmp() {
    assert_eq!(FixTag::BeginString < FixTag::BodyLength, true);
    assert_eq!(FixTag::BodyLength < FixTag::MsgType, true);
    assert_eq!(FixTag::MsgType < FixTag::SenderCompID, true);
    assert_eq!(FixTag::SenderCompID < FixTag::TargetCompID, true);
    assert_eq!(FixTag::TargetCompID < FixTag::MsgSeqNum, true);
    assert_eq!(FixTag::MsgSeqNum < FixTag::SendingTime, true);
    assert_eq!(FixTag::SendingTime < FixTag::Symbol, true);
    assert_eq!(FixTag::Symbol < FixTag::Side, true);
    assert_eq!(FixTag::Side < FixTag::OrderQty, true);
    assert_eq!(FixTag::OrderQty < FixTag::Price, true);
    assert_eq!(FixTag::Price < FixTag::OrdType, true);
    assert_eq!(FixTag::OrdType < FixTag::OrderID, true);
    assert_eq!(FixTag::OrderID < FixTag::ExecID, true);
    assert_eq!(FixTag::ExecID < FixTag::LeavesQty, true);
    assert_eq!(FixTag::LeavesQty < FixTag::CumQty, true);
    assert_eq!(FixTag::CumQty < FixTag::AvgPx, true);
    assert_eq!(FixTag::AvgPx < FixTag::Text, true);
    assert_eq!(FixTag::Text < FixTag::CheckSum, true);
}
