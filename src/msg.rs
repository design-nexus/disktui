use crate::model::{Caps, Model};

#[derive(Clone, Debug)]
pub struct Progress {
    pub label: String,
    pub ratio: f64,
}

pub enum Msg {
    Model(Model),
    Caps(Caps),
    Info(String),
    Error(String),
    Progress(Option<Progress>),
    Sample { pos: f64, mbps: f64 },
    Smart(String),
    ThemeReload,
    Busy(bool),
    Attached(String),
}
