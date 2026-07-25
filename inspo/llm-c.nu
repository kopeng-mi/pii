# llm-c.nu — side-by-side model comparison
use llm-api.nu *

export def llm-c [
    ...queries: string
    --refresh (-r)
] {
    let models = (get-models --refresh=$refresh)

    mut targets = []
    if ($queries | is-empty) {
        let count_str = (input $"(fmt-gray 'How many models to compare?') (fmt-dim '[2]'): ")
        let count = (if ($count_str | str trim | is-empty) { 2 } else { try { $count_str | str trim | into int } catch { 2 } })

        if $count < 1 { return }

        let lines = ($models | each { |m| $"($m.id) │ ($m.name) │ ($m.model_creator | get -o name | default '')" })

        for i in 1..$count {
            let picked = ($lines | str join "\n" | fzf --reverse --prompt=$"Select model ($i)/($count)▸ " --height=20)
            if ($picked | is-empty) {
                print $"  (fmt-dim $'Cancelled at model ($i).')"
                return
            }
            let picked_id = ($picked | split row " │ " | first | str trim)
            let m = ($models | where id == $picked_id)
            if not ($m | is-empty) {
                $targets = ($targets | append ($m | first))
            }
        }
    } else {
        for q in $queries {
            let found = (find-model $q $models)
            if $found != null {
                $targets = ($targets | append $found)
            } else {
                print $"  (fmt-rose '✗') Model not found: (fmt-dim $q)"
            }
        }
    }

    if ($targets | is-empty) { return }
    let final_targets = $targets

    # Disambiguate display names: if two models share the same name, append id
    let names = ($final_targets | get name)
    let col_names = ($final_targets | enumerate | each { |it|
        let n = $it.item.name
        let dupes = ($names | where { |x| $x == $n } | length)
        if $dupes > 1 { $"($n) [($it.item.id)]" } else { $n }
    })

    # ── Metrics ─────────────────────────────────────────────────────────
    let base_metrics = [
        ["Creator",       "info"],
        ["Released",      "info"],
        ["Context",       "info"],
        ["In ($/1M)",     "price"],
        ["Out ($/1M)",    "price"],
        ["Speed (tok/s)", "perf"],
        ["TTFT (s)",      "perf"]
    ]

    # Collect all eval keys across targets
    let eval_keys = ($final_targets | each { |t|
        $t | get -o evaluations | default {} | columns
    } | flatten | uniq)

    # ── Build rows ──────────────────────────────────────────────────────
    mut all_rows = []

    for pair in $base_metrics {
        let metric = ($pair | get 0)
        mut row = { "": (fmt-gray $metric) }
        for idx in 0..((($final_targets | length) - 1)) {
            let t = ($final_targets | get $idx)
            let col = ($col_names | get $idx)
            let val = (match $metric {
                "Creator" => (fmt-dim ($t.model_creator | get -o name | default "")),
                "Released" => (fmt-dim ($t.release_date | default "Unknown")),
                "Context" => (compact-num ($t | get -o context_window)),
                "In ($/1M)" => (format-price $t.input),
                "Out ($/1M)" => (format-price $t.output),
                "Speed (tok/s)" => (format-score ($t | get -o performance | get -o median_output_tokens_per_second | default null)),
                "TTFT (s)" => {
                    let ttft = ($t | get -o performance | get -o median_time_to_first_token_seconds | default null)
                    if $ttft != null { fmt-teal $"($ttft)" } else { fmt-muted "--" }
                },
                _ => ""
            })
            $row = ($row | upsert (fmt-bold $col) $val)
        }
        $all_rows = ($all_rows | append $row)
    }

    # Add separator row if there are benchmarks
    if not ($eval_keys | is-empty) {
        mut sep_row = { "": (fmt-teal "─ Benchmarks") }
        for idx in 0..((($final_targets | length) - 1)) {
            let col = ($col_names | get $idx)
            $sep_row = ($sep_row | upsert (fmt-bold $col) "")
        }
        $all_rows = ($all_rows | append $sep_row)

        # Compute max per benchmark for spark bars
        for key in $eval_keys {
            let scores_for_key = ($final_targets | each { |t|
                $t | get -o evaluations | default {} | get -o $key | default null
            } | where { |v| $v != null })
            let max_s = if ($scores_for_key | is-empty) { 100.0 } else {
                let m = ($scores_for_key | into float | math max)
                if $m > 100.0 { $m } else { 100.0 }
            }

            mut row = { "": (fmt-gray $key) }
            for idx in 0..((($final_targets | length) - 1)) {
                let t = ($final_targets | get $idx)
                let col = ($col_names | get $idx)
                let s = ($t | get -o evaluations | default {} | get -o $key | default null)
                let val = if $s != null {
                    (spark-bar ($s | into float) $max_s 16 -l $"($s)")
                } else {
                    fmt-muted "--"
                }
                $row = ($row | upsert (fmt-bold $col) $val)
            }
            $all_rows = ($all_rows | append $row)
        }
    }

    section "Model Comparison" -s $"($final_targets | length) models"
    print ""
    $all_rows | table -i false | print
    print ""
}
