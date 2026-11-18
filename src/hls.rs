use anyhow::{Context, Result, ensure};
use reqwest::Url;

pub fn lowest_variant(base: &Url, playlist: &str) -> Result<Url> {
    ensure!(
        playlist.trim_start().starts_with("#EXTM3U"),
        "{}",
        tf!(
            "The stream response is not an HLS playlist",
            "Ответ потока не является HLS-плейлистом"
        )
    );
    let mut variants = Vec::new();
    let mut attributes = None;
    for line in playlist.lines().map(str::trim) {
        if let Some(value) = line.strip_prefix("#EXT-X-STREAM-INF:") {
            attributes = Some(value);
        } else if !line.is_empty()
            && !line.starts_with('#')
            && let Some(attrs) = attributes.take()
        {
            let bandwidth = attrs
                .split(',')
                .find_map(|part| part.strip_prefix("BANDWIDTH="))
                .and_then(|n| n.parse::<u64>().ok())
                .unwrap_or(u64::MAX);
            // Prefer video renditions; audio-only may not count for channel points.
            let is_video = attrs.contains("RESOLUTION=")
                || (attrs.contains("VIDEO=") && !attrs.contains("audio_only"));
            variants.push((
                !is_video,
                bandwidth,
                base.join(line).context(tr!(
                    "Invalid HLS quality URL",
                    "Неверный адрес качества HLS"
                ))?,
            ));
        }
    }
    variants.sort_by_key(|(audio, bandwidth, _)| (*audio, *bandwidth));
    variants
        .into_iter()
        .next()
        .map(|(_, _, url)| url)
        .context(tr!(
            "No available qualities in the HLS playlist",
            "В HLS нет доступных качеств"
        ))
}

pub fn latest_segment(base: &Url, playlist: &str) -> Result<Url> {
    ensure!(
        playlist.trim_start().starts_with("#EXTM3U"),
        "{}",
        tf!(
            "The stream response is not an HLS playlist",
            "Ответ потока не является HLS-плейлистом"
        )
    );
    let line = playlist
        .lines()
        .map(str::trim)
        .rfind(|line| !line.is_empty() && !line.starts_with('#'))
        .context(tr!(
            "No HLS segments; the stream may have ended",
            "В HLS нет сегментов; канал мог завершить эфир"
        ))?;
    base.join(line).context(tr!(
        "Invalid HLS segment URL",
        "Неверный адрес сегмента HLS"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chooses_lowest_video_even_if_audio_is_cheaper() {
        let base = Url::parse("https://example.test/live/master.m3u8").unwrap();
        let text = "#EXTM3U\r\n#EXT-X-STREAM-INF:BANDWIDTH=100,CODECS=\"mp4a.40.2\",VIDEO=\"audio_only\"\r\naudio.m3u8\r\n#EXT-X-STREAM-INF:BANDWIDTH=2000,RESOLUTION=1920x1080\r\nhigh.m3u8\r\n#EXT-X-STREAM-INF:BANDWIDTH=300,RESOLUTION=284x160\r\nlow.m3u8\r\n";
        assert_eq!(
            lowest_variant(&base, text).unwrap().as_str(),
            "https://example.test/live/low.m3u8"
        );
    }
    #[test]
    fn accepts_relative_segments_and_trailing_blank_lines() {
        let base = Url::parse("https://example.test/live/low.m3u8").unwrap();
        let text = "#EXTM3U\n#EXTINF:2,\nold.ts\n#EXTINF:2,\nnew.ts?key=foo\n\n#EXT-X-ENDLIST\n";
        assert_eq!(
            latest_segment(&base, text).unwrap().as_str(),
            "https://example.test/live/new.ts?key=foo"
        );
        assert!(latest_segment(&base, "#EXTM3U\n").is_err());
        assert!(lowest_variant(&base, "<html>error</html>").is_err());
    }
}
