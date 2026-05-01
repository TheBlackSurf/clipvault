use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub retention_days: Option<i64>,
    pub max_items: i64,
    pub shortcut: String,
    pub paused: bool,
    pub hide_after_copy: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            retention_days: Some(7),
            max_items: 500,
            shortcut: "CommandOrControl+Shift+S".to_string(),
            paused: false,
            hide_after_copy: true,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct SourceMetadata {
    pub app_name: Option<String>,
    pub window_title: Option<String>,
    pub page_url: Option<String>,
    pub page_title: Option<String>,
    pub domain: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NewClipboardItem {
    pub kind: String,
    pub content: String,
    pub url: Option<String>,
    pub domain: Option<String>,
    pub title: Option<String>,
    pub source: SourceMetadata,
    pub hash: String,
    pub blob: Option<Vec<u8>>,
    pub thumb: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipboardItem {
    pub id: i64,
    pub kind: String,
    pub content: String,
    pub url: Option<String>,
    pub domain: Option<String>,
    pub title: Option<String>,
    pub source_app: Option<String>,
    pub source_url: Option<String>,
    pub source_title: Option<String>,
    pub source_domain: Option<String>,
    pub created_at: i64,
    pub thumb_data_url: Option<String>,
}
