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
    let only_chinese = Transcript::from_cues(&subtitles::parse("只有中文\n这里有 Example 译文"));
    ensure!(
        only_chinese.text.is_empty() && only_chinese.html.is_empty(),
        "没有英文时没有清空"
    );
    ensure!(Transcript::default().text.is_empty(), "新视频含旧文稿");

    let second_video = root.join("second.mp4");
    fs::write(&second_video, b"second isolated identity")?;
    ensure!(
        Transcript::from_cues(&video_subtitles::load(&lib, &second_video)?)
            .text
            .is_empty(),
        "换片残留文稿"
    );
    let replacement =
        subtitles::parse("WEBVTT\n\n00:00.000 --> 00:01.000\nReplacement sentence.\n替换翻译\n");
    video_subtitles::save(&lib, &video, &replacement, &process::cancel_token())?;
    ensure!(
        Transcript::from_cues(&video_subtitles::load(&lib, &video)?).text
            == "Replacement sentence.",
        "重生成未更新全文"
    );
    let english_only =
        subtitles::parse("WEBVTT\n\n00:00.000 --> 00:01.000\nReplacement sentence.\n");
    ensure!(
        Transcript::from_cues(&english_only).text == Transcript::from_cues(&replacement).text,
        "翻译改变了英文文稿"
    );
    let long = (0..10_000)
        .map(|i| subtitles::Cue {
            start: i as f64,
            end: i as f64 + 1.,
            text: format!("Sentence {i}.\n第{i}句"),
        })
        .collect::<Vec<_>>();
    let full = Transcript::from_cues(&long);
    ensure!(
        full.text.starts_with("Sentence 0.")
            && full.text.ends_with("Sentence 9999.")
            && full.text.matches("Sentence").count() == 10_000,
        "长字幕被截断"
    );
    fs::write(root.join("long-english.txt"), &full.text)?;
    fs::write(
        root.join("checks.json"),
        serde_json::to_vec_pretty(
            &json!({"passed":true,"bilingual_reload":true,"repeat_preserved":true,"escaped_html":safe.html,"switch_cleared":true,"replacement_updated":true,"translation_stable":true,"long_cues":10_000,"long_characters":full.text.chars().count()}),
        )?,
    )?;
    results.push(json!({"workflow":"视频字幕保存重开 → 整篇纯英文／选区 HTML／复制文本 → 换片及重生成 → 一万句完整文稿","passed":true,"evidence":root}));
    Ok(())
}
