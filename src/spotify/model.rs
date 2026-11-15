use serde_json::Value;

#[derive(Clone, Debug, Default)]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub detail: String,
    pub uri: String,
    pub image: String,
    pub position: usize,
    pub playable: bool,
}

#[derive(Default)]
pub struct Page {
    pub entries: Vec<Entry>,
    pub next: Option<String>,
}

pub fn image(value: &Value) -> String {
    value
        .as_array()
        .and_then(|images| {
            images
                .iter()
                .find(|image| image["width"].as_u64().unwrap_or(300) >= 300)
                .or(images.first())
        })
        .and_then(|image| image["url"].as_str())
        .unwrap_or_default()
        .into()
}

pub fn artists(value: &Value) -> String {
    value
        .as_array()
        .map(|artists| {
            artists
                .iter()
                .filter_map(|artist| artist["name"].as_str())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default()
}

fn text(value: &Value) -> String {
    value.as_str().unwrap_or_default().into()
}

pub fn playlists(value: &Value) -> Page {
    let entries = value["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|item| item.is_object())
        .map(|item| Entry {
            id: text(&item["id"]),
            name: text(&item["name"]),
            detail: item["owner"]["display_name"]
                .as_str()
                .map(|name| format!("Playlist · {name}"))
                .unwrap_or_else(|| "Playlist".into()),
            uri: text(&item["uri"]),
            image: image(&item["images"]),
            playable: true,
            ..Entry::default()
        })
        .collect();
    Page {
        entries,
        next: value["next"].as_str().map(str::to_owned),
    }
}

pub fn tracks(value: &Value) -> Page {
    let offset = value["offset"].as_u64().unwrap_or(0) as usize;
    let entries = value["items"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
        .filter_map(|(index, wrapper)| {
            // New development-mode shape uses `item`; older extended-quota responses use `track`.
            let item = wrapper.get("item").or_else(|| wrapper.get("track"))?;
            if !item.is_object() {
                return None;
            }
            let duration = item["duration_ms"].as_u64().unwrap_or(0) / 1000;
            let is_local = wrapper["is_local"].as_bool().unwrap_or(false)
                || item["is_local"].as_bool().unwrap_or(false);
            Some(Entry {
                id: text(&item["id"]),
                name: text(&item["name"]),
                uri: text(&item["uri"]),
                detail: format!(
                    "{} · {}:{:02}",
                    artists(&item["artists"]),
                    duration / 60,
                    duration % 60
                ),
                image: image(&item["album"]["images"]),
                position: offset + index,
                playable: !is_local
                    && item["is_playable"].as_bool() != Some(false)
                    && item["uri"].as_str().is_some(),
            })
        })
        .collect();
    Page {
        entries,
        next: value["next"].as_str().map(str::to_owned),
    }
}

pub fn devices(value: &Value) -> Vec<Entry> {
    value["devices"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|item| Entry {
            id: text(&item["id"]),
            name: text(&item["name"]),
            detail: format!(
                "{}{}",
                item["type"].as_str().unwrap_or("Device"),
                if item["is_active"].as_bool().unwrap_or(false) {
                    " · Active"
                } else {
                    ""
                }
            ),
            playable: item["is_restricted"].as_bool() != Some(true)
                && item["id"].as_str().is_some(),
            ..Entry::default()
        })
        .collect()
}

#[derive(Clone, Debug, Default)]
pub struct Playback {
    pub name: String,
    pub artist: String,
    pub album: String,
    pub image: String,
    pub uri: String,
    pub device: String,
    pub playing: bool,
    pub position_ms: u64,
    pub duration_ms: u64,
}

impl Playback {
    pub fn parse(value: &Value) -> Self {
        let item = &value["item"];
        Self {
            name: text(&item["name"]),
            artist: artists(&item["artists"]),
            album: text(&item["album"]["name"]),
            image: image(
                item.get("album")
                    .map(|album| &album["images"])
                    .unwrap_or(&item["images"]),
            ),
            uri: text(&item["uri"]),
            device: text(&value["device"]["name"]),
            playing: value["is_playing"].as_bool().unwrap_or(false),
            position_ms: value["progress_ms"].as_u64().unwrap_or(0),
            duration_ms: item["duration_ms"].as_u64().unwrap_or(0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn unavailable_tracks_keep_original_playback_offsets() {
        let page = tracks(&json!({"offset": 50, "items": [
            {"item": null}, {"item": {"name":"A", "uri":"spotify:track:a", "duration_ms":61000}},
            {"track": {"name":"Local", "uri":"spotify:local:a", "is_local":true}}
        ]}));
        assert_eq!(page.entries.len(), 2);
        assert_eq!(page.entries[0].position, 51);
        assert!(page.entries[0].playable);
        assert!(!page.entries[1].playable);
    }
}
