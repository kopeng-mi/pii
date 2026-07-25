# llm-s.nu — single model detail card
use llm-api.nu *

export def llm-s [
    query?: string
    --refresh (-r)
] {
    let models = (get-models --refresh=$refresh)

    let target = if $query == null {
        let lines = ($models | each { |m| $"($m.id) │ ($m.name) │ ($m.model_creator | get -o name | default '')" })
        let picked = ($lines | str join "\n" | fzf --reverse --prompt="Select model▸ " --height=20)
        if ($picked | is-empty) { return }
        let picked_id = ($picked | split row " │ " | first | str trim)
        ($models | where id == $picked_id | first)
    } else {
        let found = (find-model $query $models)
        if $found == null {
            print $"  (fmt-rose '✗') Model not found: (fmt-dim $query)"
            return
        }
        $found
    }

    if $target == null { return }

    let creator = ($target.model_creator | get -o name | default "Unknown")
    let release = ($target.release_date | default "Unknown")
    let ctx = ($target | get -o context_window)

    let in_p = $target.input
    let out_p = $target.output

    let perf = ($target | get -o performance)
    let speed = ($perf | get -o median_output_tokens_per_second | default null)
    let ttft = ($perf | get -o median_time_to_first_token_seconds | default null)

    let ev = ($target | get -o evaluations | default {})

    # ── Header ──────────────────────────────────────────────────────────
    section $target.name -s $"by ($creator)"
    print $"  (fmt-dim $target.id)  ·  (fmt-dim $release)  ·  ctx (compact-num $ctx)"
    print ""

    # ── Pricing ─────────────────────────────────────────────────────────
    print $"  (fmt-gray 'Pricing')      (fmt-gray 'In')  (format-price $in_p)   (fmt-gray 'Out')  (format-price $out_p)  (fmt-gray '/1M tok')"

    # ── Performance ─────────────────────────────────────────────────────
    let speed_str = if $speed != null { fmt-teal $"($speed) tok/s" } else { fmt-muted "--" }
    let ttft_str = if $ttft != null { fmt-teal $"($ttft)s" } else { fmt-muted "--" }
    print $"  (fmt-gray 'Performance')  (fmt-gray 'Speed')  ($speed_str)   (fmt-gray 'TTFT')  ($ttft_str)"
    print ""

    # ── Benchmarks ──────────────────────────────────────────────────────
    let ev_cols = ($ev | columns)
    if not ($ev_cols | is-empty) {
        print $"  (fmt-gray 'Benchmarks')"

        # Find max score for spark bars (assume 0–100 scale, cap at actual max)
        let scores_list = ($ev_cols | each { |c| $ev | get $c } | where { |v| $v != null })
        let max_score = if ($scores_list | is-empty) { 100.0 } else {
            let m = ($scores_list | into float | math max)
            if $m > 100.0 { $m } else { 100.0 }
        }

        for col in $ev_cols {
            let s = ($ev | get $col)
            if $s != null {
                let label = ($col | fill -w 24 -a l)
                let bar = (spark-bar ($s | into float) $max_score 20 -l $"($s)")
                print $"    (fmt-dim $label) ($bar)"
            }
        }
    }
    print ""
}
