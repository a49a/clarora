//! Persisted video subtitles → full English reading/copy cache → reload/replacement.
use anyhow::{Result, ensure};
use clarora_gpui::{
    data::Library, process, subtitles, video_subtitles, video_transcript::Transcript,
};
use serde_json::{Value, json};
use std::fs;

pub fn run(parent: &Library, results: &mut Vec<Value>) -> Result<()> {
    let root = parent.root.join("english-transcript");
    let lib = Library::open(root.join("library"))?;
    let video = root.join("first.mp4");
    fs::write(&video, b"isolated video identity fixture")?;
    let srt = "1\n00:00:01,000 --> 00:00:02,000\nHello &amp; welcome.\n你好，欢迎。\n\n2\n00:00:02,000 --> 00:00:04,000\nKeep learning\nevery day.\n每天学习。\n\n3\n00:00:04,000 --> 00:00:05,000\nKeep learning\nevery day.\n\n4\n00:00:05,000 --> 00:00:06,000\n只有中文\n";
    fs::write(root.join("bilingual.srt"), srt)?;
    let cues = subtitles::parse(srt);
    video_subtitles::save(&lib, &video, &cues, &process::cancel_token())?;
    let reopened = Library::open(lib.root.clone())?;
    let loaded = video_subtitles::load(&reopened, &video)?;
    let transcript = Transcript::from_cues(&loaded);
    let expected = "Hello & welcome.\n\nKeep learning every day.\n\nKeep learning every day.";
    ensure!(transcript.text == expected, "双语全文、实体或重复语句错误");
    ensure!(
        (
            transcript.cue_count,
            transcript.word_count,
            transcript.character_count
        ) == (3, 11, 68),
        "双语重开后的英文缓存统计不正确"
    );
    ensure!(
        transcript.html
            == "<p>Hello &amp; welcome.</p><p>Keep learning every day.</p><p>Keep learning every day.</p>",
        "显示文稿与复制正文不一致"
    );
    let mut statistics = vec![
        json!({"case":"bilingual-reopened","cues":transcript.cue_count,"words":transcript.word_count,"characters":transcript.character_count}),
    ];
    ensure!(
        !transcript.html.contains("你好") && !transcript.html.contains("-->"),
        "文稿混入翻译或时间轴"
    );
    fs::write(root.join("english.txt"), &transcript.text)?;
    fs::write(root.join("english.html"), &transcript.html)?;

    let unsafe_cues = vec![subtitles::Cue {
        start: 0.,
        end: 1.,
        text: "Use <script>alert('x')</script> & enjoy.\n中文 Example".into(),
    }];
    let safe = Transcript::from_cues(&unsafe_cues);
    ensure!(
        safe.text == "Use <script>alert('x')</script> & enjoy.",
        "复制文本被 HTML 改写或混入译文"
    );
    ensure!(
        !safe.html.contains("<script>")
            && safe.html.contains("&lt;script&gt;")
            && safe.html.contains("&amp;"),
        "HTML 未转义"
    );
    ensure!(
        (safe.cue_count, safe.word_count, safe.character_count) == (1, 4, 40),
        "HTML 转义改变了复制缓存的统计"
    );
    let only_chinese = Transcript::from_cues(&subtitles::parse("只有中文\n这里有 Example 译文"));
    ensure!(
        only_chinese.text.is_empty()
            && only_chinese.html.is_empty()
            && (
                only_chinese.cue_count,
                only_chinese.word_count,
                only_chinese.character_count
            ) == (0, 0, 0),
        "没有英文时没有清空"
    );
    let empty = Transcript::default();
    ensure!(
        empty.text.is_empty()
            && empty.html.is_empty()
            && (empty.cue_count, empty.word_count, empty.character_count) == (0, 0, 0),
        "新视频含旧文稿或统计"
    );
    statistics.push(json!({"case":"default-and-filtered-empty","cues":0,"words":0,"characters":0}));

    let second_video = root.join("second.mp4");
    fs::write(&second_video, b"second isolated identity")?;
    let second = Transcript::from_cues(&video_subtitles::load(&lib, &second_video)?);
    ensure!(
        second.text.is_empty()
            && (second.cue_count, second.word_count, second.character_count) == (0, 0, 0),
        "换片残留文稿"
    );
    let replacement =
        subtitles::parse("WEBVTT\n\n00:00.000 --> 00:01.000\nReplacement sentence.\n替换翻译\n");
    video_subtitles::save(&lib, &video, &replacement, &process::cancel_token())?;
    let replacement_library = Library::open(lib.root.clone())?;
    let replaced = Transcript::from_cues(&video_subtitles::load(&replacement_library, &video)?);
    ensure!(
        replaced.text == "Replacement sentence."
            && (
                replaced.cue_count,
                replaced.word_count,
                replaced.character_count
            ) == (1, 2, 21),
        "重生成未更新全文"
    );
    let english_only =
        subtitles::parse("WEBVTT\n\n00:00.000 --> 00:01.000\nReplacement sentence.\n");
    let english_only = Transcript::from_cues(&english_only);
    ensure!(
        english_only.text == replaced.text
            && (
                english_only.cue_count,
                english_only.word_count,
                english_only.character_count
            ) == (1, 2, 21),
        "翻译改变了英文文稿"
    );
    statistics.push(json!({"case":"replacement-reopened-and-translation-stable","cues":replaced.cue_count,"words":replaced.word_count,"characters":replaced.character_count}));

    let unicode_srt = "1\n00:00:00,000 --> 00:00:01,000\nWe learn daily&#x1F331;.\n每天学习。\n\n2\n00:00:01,000 --> 00:00:02,000\nCafé &amp; tea &lt;3.\n咖啡和茶。\n\n3\n00:00:02,000 --> 00:00:03,000\n这是 Example 混排译文。\n";
    fs::write(root.join("unicode.srt"), unicode_srt)?;
    let unicode_cues = subtitles::parse(unicode_srt);
    video_subtitles::save(&lib, &second_video, &unicode_cues, &process::cancel_token())?;
    let unicode_library = Library::open(lib.root.clone())?;
    let unicode = Transcript::from_cues(&video_subtitles::load(&unicode_library, &second_video)?);
    ensure!(
        unicode.text == "We learn daily🌱.\n\nCafé & tea <3."
            && unicode.html == "<p>We learn daily🌱.</p><p>Café &amp; tea &lt;3.</p>"
            && (
                unicode.cue_count,
                unicode.word_count,
                unicode.character_count
            ) == (2, 7, 32)
            && unicode.text.len() > unicode.character_count,
        "实体、Unicode 字符或 HTML/复制缓存统计不一致"
    );
    fs::write(root.join("unicode-english.txt"), &unicode.text)?;
    fs::write(root.join("unicode-english.html"), &unicode.html)?;
    statistics.push(json!({"case":"unicode-entities-reopened","cues":unicode.cue_count,"words":unicode.word_count,"characters":unicode.character_count,"utf8_bytes":unicode.text.len()}));
    let long = (0..10_000)
        .map(|i| subtitles::Cue {
            start: i as f64,
            end: i as f64 + 1.,
            text: format!("Sentence {i}.\n第{i}句"),
        })
        .collect::<Vec<_>>();
    video_subtitles::save(&lib, &video, &long, &process::cancel_token())?;
    let long_library = Library::open(lib.root.clone())?;
    let full = Transcript::from_cues(&video_subtitles::load(&long_library, &video)?);
    ensure!(
        full.text.starts_with("Sentence 0.")
            && full.text.ends_with("Sentence 9999.")
            && full.text.matches("Sentence").count() == 10_000
            && (full.cue_count, full.word_count, full.character_count) == (10_000, 20_000, 158_888),
        "长字幕被截断"
    );
    fs::write(root.join("long-english.txt"), &full.text)?;
    statistics.push(json!({"case":"ten-thousand-reopened","cues":full.cue_count,"words":full.word_count,"characters":full.character_count}));

    video_subtitles::save(&lib, &video, &replacement, &process::cancel_token())?;
    let short_library = Library::open(lib.root.clone())?;
    let short = Transcript::from_cues(&video_subtitles::load(&short_library, &video)?);
    ensure!(
        short.text == "Replacement sentence."
            && (short.cue_count, short.word_count, short.character_count) == (1, 2, 21)
            && full.cue_count == 10_000,
        "短字幕继承旧长文稿统计或更改了已构建缓存"
    );
    statistics.push(json!({"case":"long-to-short-reopened","cues":short.cue_count,"words":short.word_count,"characters":short.character_count}));
    fs::write(
        root.join("checks.json"),
        serde_json::to_vec_pretty(
            &json!({"passed":true,"bilingual_reload":true,"repeat_preserved":true,"escaped_html":safe.html,"switch_cleared":true,"replacement_updated":true,"translation_stable":true,"long_cues":10_000,"long_characters":full.character_count,"cached_statistics":statistics}),
        )?,
    )?;
    results.push(json!({"workflow":"视频字幕保存重开 → 整篇纯英文／选区 HTML／复制文本与缓存统计 → 换片及重生成 → 一万句完整文稿","passed":true,"evidence":root}));
    Ok(())
}
