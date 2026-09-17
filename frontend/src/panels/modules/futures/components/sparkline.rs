use leptos::prelude::*;

pub(in crate::panels::modules::futures) fn sparkline(points: Vec<f64>) -> impl IntoView {
    let (width, height): (u32, u32) = (80, 22);
    let points = finite_points(points);
    if points.len() < 2 {
        return view! { <span class="num muted">"趋势待历史样本"</span> }.into_any();
    }

    let (min, max) = points
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(min, max), value| {
            (min.min(*value), max.max(*value))
        });
    let range = (max - min).max(1e-9);
    let step = width as f64 / (points.len().saturating_sub(1) as f64).max(1.0);
    let path = points
        .iter()
        .enumerate()
        .map(|(idx, value)| {
            let x = idx as f64 * step;
            let y = height as f64 - ((*value - min) / range) * height as f64;
            format!("{} {:.1} {:.1}", if idx == 0 { "M" } else { "L" }, x, y)
        })
        .collect::<Vec<_>>()
        .join(" ");
    let stroke = if points.last() >= points.first() {
        "var(--color-accent)"
    } else {
        "var(--color-danger)"
    };

    view! {
        <svg class="sparkline" width=width height=height viewBox=format!("0 0 {width} {height}")>
            <path d=path fill="none" stroke=stroke stroke-width="1.5"/>
        </svg>
    }
    .into_any()
}

fn finite_points(points: Vec<f64>) -> Vec<f64> {
    points
        .into_iter()
        .filter(|value| value.is_finite())
        .collect()
}
