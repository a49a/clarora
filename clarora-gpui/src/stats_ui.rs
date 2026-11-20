//! Statistics page cards, charts and distraction-rate summary.
use super::*;

pub(super) fn content(app: &AppView, cx: &mut Context<AppView>) -> Div {
    let mut content = div();
    let rows = &app.data.stats;
    let range = app.stats_range;
    let days = match range {
        0 => 1,
        1 => 7,
        2 => 14,
        _ => 0,
    };
    let today = clarora_gpui::data::current_study_date();
    let in_range = |day: &str| {
        range == 3
            || chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d")
                .is_ok_and(|date| date <= today && date > today - chrono::Duration::days(days))
    };
    let focus_minutes: f64 = if range == 3 {
        app.data.focus_minutes
    } else {
        app.data
            .focus_by_day
            .iter()
            .filter(|(day, _)| in_range(day))
            .map(|(_, minutes)| *minutes)
            .sum()
    };
    let focus_distractions: u64 = if range == 3 {
        app.data.focus_distractions
    } else {
        app.data
            .focus_distractions_by_day
            .iter()
            .filter(|(day, _)| in_range(day))
            .map(|(_, count)| *count)
            .sum()
    };
    let listening: f64 = rows
        .iter()
        .filter(|row| in_range(&text(row, "day")))
        .filter_map(|r| r["listen_seconds"].as_f64())
        .sum();
    let review: f64 = rows
        .iter()
        .filter(|row| in_range(&text(row, "day")))
        .filter_map(|r| r["review_count"].as_f64())
        .sum();
    let pomodoro: f64 = rows
        .iter()
        .filter(|row| in_range(&text(row, "day")))
        .filter_map(|r| r["pomodoro_count"].as_f64())
        .sum();
    let range_label = match range {
        0 => "今日",
        1 => "近 7 天",
        2 => "近 14 天",
        _ => "累计",
    };
    let average_interval = if focus_distractions == 0 {
        "暂无分心".to_string()
    } else {
        format!("{:.1} 分钟", focus_minutes / focus_distractions as f64)
    };
    let mut ranges = div().flex().gap_2().flex_wrap();
    for (value, label) in [(0, "今日"), (1, "近 7 天"), (2, "近 14 天"), (3, "全部")] {
        ranges = ranges.child(
            app.button(
                format!("stats-range-{value}"),
                label,
                Action::StatsRange(value),
                cx,
            )
            .small()
            .outline()
            .selected(range == value),
        );
    }
    content = content.child(ranges).child(
        div()
            .flex()
            .gap_4()
            .flex_wrap()
            .child(
                panel(cx)
                    .child(format!("{range_label}专注"))
                    .child(format!("{focus_minutes:.1} 分钟")),
            )
            .child(
                panel(cx)
                    .child(format!("{range_label}平均分心间隔"))
                    .child(average_interval),
            )
            .child(
                panel(cx)
                    .child(format!("{range_label}媒体播放"))
                    .child(format!("{:.1} 分钟", listening / 60.)),
            )
            .child(
                panel(cx)
                    .child(format!("{range_label}复习卡片"))
                    .child(format!("{review:.0} 张")),
            )
            .child(
                panel(cx)
                    .child(format!("{range_label}完成番茄钟"))
                    .child(format!("{pomodoro:.0} 轮")),
            ),
    );
    let mut heatmap = div().flex().gap_1().flex_wrap().max_w(px(650.));
    for day in (0..119).rev() {
        let date = (today - chrono::Duration::days(day))
            .format("%Y-%m-%d")
            .to_string();
        let count = rows
            .iter()
            .find(|r| text(r, "day") == date)
            .and_then(|r| r["review_count"].as_f64())
            .unwrap_or(0.);
        heatmap = heatmap.child(div().size(px(20.)).rounded_sm().bg(rgb(if count > 30. {
            0x17604c
        } else if count > 10. {
            0x459d7a
        } else if count > 0. {
            0x9ed6b5
        } else {
            0xe6e9e4
        })));
    }
    content = content.child("最近 17 周复习热力图").child(heatmap);
    let days = (0..14)
        .rev()
        .map(|day| {
            (today - chrono::Duration::days(day))
                .format("%Y-%m-%d")
                .to_string()
        })
        .collect::<Vec<_>>();
    let media_days = days
        .iter()
        .map(|date| {
            let seconds = rows
                .iter()
                .find(|r| text(r, "day") == *date)
                .and_then(|r| r["listen_seconds"].as_f64())
                .unwrap_or(0.);
            (date.clone(), seconds / 60.)
        })
        .collect::<Vec<_>>();
    let focus_days = days
        .iter()
        .map(|date| {
            (
                date.clone(),
                app.data.focus_by_day.get(date).copied().unwrap_or(0.),
            )
        })
        .collect::<Vec<_>>();
    content = content
        .child("最近 14 天专注时长")
        .child(daily_minutes_chart(cx, &focus_days, 10.))
        .child("最近 14 天媒体播放时长")
        .child(daily_minutes_chart(cx, &media_days, 10.))
        .child("各音频累计播放时长（前 10）")
        .child(audio_playback_stats(app, cx))
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("各音频累计播放时长不受上方统计范围影响；删除音频后对应记录会一并移除。学习日按凌晨 4 点切换，0–4 点的记录归前一天。专注时长按每轮「结束本轮」的实际时长统计；媒体播放只累计未暂停的音频或视频。阅读、写作、口语、暂停和冥想不计入媒体播放。平均分心间隔 = 所选范围内已结束专注轮次的实际总时长 ÷ 分心总次数；上方卡片随范围切换。"),
        );
    let attempts = &app.data.speaking;
    let mut trend = div().flex().gap_2().items_end().h(px(140.));
    for record in attempts.iter().rev() {
        let accuracy = record["score"]["accuracy"]
            .as_f64()
            .or_else(|| record["accuracy"].as_f64().map(|v| v * 100.))
            .unwrap_or(0.);
        trend = trend.child(
            div()
                .flex()
                .flex_col()
                .justify_end()
                .items_center()
                .gap_1()
                .child(format!("{accuracy:.0}%"))
                .child(
                    div()
                        .w(px(20.))
                        .h(px(accuracy.clamp(2., 100.) as f32))
                        .bg(cx.theme().primary),
                ),
        );
    }
    content = content.child("最近 20 次跟读准确度").child(trend);

    for row in rows.iter().rev().take(14) {
        content = content.child(format!(
            "{} · 复习 {} · 媒体 {:.1} 分钟",
            text(row, "day"),
            text(row, "review_count"),
            row["listen_seconds"].as_f64().unwrap_or(0.) / 60.
        ));
    }
    content
}

fn audio_playback_stats(app: &AppView, cx: &Context<AppView>) -> Div {
    if app.data.audio_stats.is_empty() {
        return div()
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child("暂无音频播放记录。");
    }
    let maximum = app
        .data
        .audio_stats
        .iter()
        .filter_map(|row| row["seconds"].as_f64())
        .fold(0., f64::max);
    let mut list = div().flex().flex_col().gap_2().max_w(px(680.));
    for row in app.data.audio_stats.iter().take(10) {
        let name = row["name"].as_str().unwrap_or("未命名音频");
        let seconds = row["seconds"].as_f64().unwrap_or(0.);
        let ratio = if maximum > 0. {
            (seconds / maximum).clamp(0., 1.) as f32
        } else {
            0.
        };
        list = list.child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_sm()
                        .truncate()
                        .child(name.to_string()),
                )
                .child(
                    div()
                        .w(px(100.))
                        .flex_shrink_0()
                        .h(px(8.))
                        .rounded_full()
                        .bg(cx.theme().muted)
                        .child(
                            div()
                                .h_full()
                                .w(relative(ratio))
                                .rounded_full()
                                .bg(cx.theme().primary),
                        ),
                )
                .child(
                    div()
                        .w(px(72.))
                        .text_right()
                        .text_sm()
                        .child(format!("{:.1} 分钟", seconds / 60.)),
                ),
        );
    }
    list
}

fn daily_minutes_chart(cx: &Context<AppView>, days: &[(String, f64)], floor: f64) -> Div {
    let maximum = days
        .iter()
        .map(|(_, minutes)| *minutes)
        .fold(floor, f64::max);
    let mut chart = div().flex().gap_1().items_end().h(px(140.));
    for (day, minutes) in days {
        chart = chart.child(
            div()
                .flex()
                .flex_col()
                .justify_end()
                .items_center()
                .w(px(45.))
                .gap_1()
                .child(format!("{:.0}m", minutes))
                .child(
                    div()
                        .w(px(24.))
                        .h(px(if *minutes > 0. {
                            (minutes / maximum * 100.).max(2.) as f32
                        } else {
                            0.
                        }))
                        .bg(cx.theme().primary),
                )
                .child(day[5..].to_string()),
        );
    }
    chart
}
