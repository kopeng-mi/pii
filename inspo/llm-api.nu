# llm-api.nu — shared data layer + formatters for llm-s / llm-c

# ── Color palette ───────────────────────────────────────────────────────────
# Teal accent: 38;5;43   Dim teal: 38;5;30   Muted: 38;5;242
# Amber/gold:  38;5;220  Rose/err: 38;5;196  Green: 38;5;114
# White bold:  1;38;5;255  Mid gray: 38;5;246  Dark gray: 38;5;237

# ── Formatting ──────────────────────────────────────────────────────────────

export def fmt-dim [text: string] { $"\e[38;5;242m($text)\e[0m" }
export def fmt-muted [text: string] { $"\e[38;5;237m($text)\e[0m" }
export def fmt-teal [text: string] { $"\e[38;5;43m($text)\e[0m" }
export def fmt-gold [text: string] { $"\e[38;5;220m($text)\e[0m" }
export def fmt-bold [text: string] { $"\e[1;38;5;255m($text)\e[0m" }
export def fmt-rose [text: string] { $"\e[38;5;196m($text)\e[0m" }
export def fmt-gray [text: string] { $"\e[38;5;246m($text)\e[0m" }

export def format-price [price] {
    if $price == null or $price == 0.0 { fmt-muted "--" } else { fmt-gold $"$($price)" }
}

export def format-score [score] {
    if $score == null { fmt-muted "--" } else { fmt-teal $"($score)" }
}

# Compact number: 128000 → 128K, 1500000 → 1.5M
export def compact-num [n] {
    if $n == null { return (fmt-muted "--") }
    let v = ($n | into float)
    if $v >= 1_000_000 {
        let r = ($v / 1_000_000 | math round --precision 1)
        $"($r | into string | str replace '.0' '')M"
    } else if $v >= 1_000 {
        let r = ($v / 1_000 | math round --precision 1)
        $"($r | into string | str replace '.0' '')K"
    } else {
        $"($n)"
    }
}

# Solid bar with value rendered inside using contrasting colors
# Filled: bright bg + dark text. Empty: dark bg + dim text.
export def spark-bar [val, max_val, width: int = 16, --label (-l): string] {
    if $val == null {
        let empty = (1..$width | each { " " } | str join "")
        return $"\e[48;5;236;38;5;242m($empty)\e[0m"
    }
    let text = (if $label != null and $label != "" { $" ($label)" } else { $" ($val)" })
    if $max_val == null or $max_val == 0 or $val == 0 {
        let padded = ($text | fill -w $width -a l)
        return $"\e[48;5;236;38;5;242m($padded)\e[0m"
    }
    let ratio = (($val | into float) / ($max_val | into float))
    let filled = ($ratio * $width | math round | into int)
    let filled = (if $filled > $width { $width } else if $filled < 1 { 1 } else { $filled })
    let empty = $width - $filled

    let padded = ($text | fill -w $width -a l)
    let text_filled = ($padded | str substring 0..$filled)
    let text_empty = ($padded | str substring $filled..$width)

    let bg_fill = (if $ratio >= 0.7 { "48;5;43" } else if $ratio >= 0.4 { "48;5;30" } else { "48;5;23" })
    $"\e[($bg_fill);38;5;232m($text_filled)\e[48;5;236;38;5;246m($text_empty)\e[0m"
}

# Section header with box-drawing line
export def section [title: string, --sub (-s): string] {
    let suffix = if $sub != null and $sub != "" { $" \e[38;5;246m($sub)\e[0m" } else { "" }
    print $"\n  \e[38;5;43m━━\e[0m (fmt-bold $title)($suffix)"
}

# ── Data fetching ───────────────────────────────────────────────────────────

export def get-models [
    --refresh (-r)
] {
    let db_path = ($nu.default-config-dir | path join "llm-stats.db")
    let today = (date now | format date "%Y-%m-%d")

    if not $refresh and ($db_path | path exists) {
        let db_cols = (try { open $db_path | columns } catch { [] })
        if "cache" in $db_cols {
            let db = (open $db_path)
            let latest = ($db.cache | last)
            if $latest.date == $today {
                return ($latest.payload | from json)
            }
        }
    }

    let aa_key = $env.ARTIFICIALANALYSIS_API_KEY? | default ""
    let ls_key = $env.LLM_STATS_API_KEY? | default ""

    if $ls_key == "" and $aa_key == "" {
        error make { msg: "Set LLM_STATS_API_KEY or ARTIFICIALANALYSIS_API_KEY." }
    }

    print $"  (fmt-teal '⟳') Fetching models..."

    mut mapped = []

    if $ls_key != "" {
        let res = (try {
            http get -H { "Authorization": $"Bearer ($ls_key)" } "https://api.llm-stats.com/stats/v1/models?limit=200"
        } catch { |e| null })

        if $res != null and ($res | get -o models | is-not-empty) {
            $mapped = ($res.models | each { |m|
                let provs = ($m | get -o providers | default [])
                let best_prov = if not ($provs | is-empty) {
                    $provs | sort-by -c input_price_per_m | first
                } else { null }

                let in_p = (if $best_prov != null { $best_prov | get -o input_price_per_m | default 0.0 } else { 0.0 })
                let out_p = (if $best_prov != null { $best_prov | get -o output_price_per_m | default 0.0 } else { 0.0 })
                let scores = ($m | get -o top_scores | default {})

                {
                    id: $m.id,
                    name: $m.name,
                    model_creator: { name: ($m | get -o organization.name | default "Unknown") },
                    release_date: ($m | get -o release_date | default "Unknown"),
                    input: $in_p,
                    output: $out_p,
                    context_window: ($m | get -o context_window | default null),
                    performance: {
                        median_output_tokens_per_second: null,
                        median_time_to_first_token_seconds: null
                    },
                    evaluations: $scores
                }
            })
        }
    }

    if ($mapped | is-empty) and $aa_key != "" {
        let res = (try {
            http get -H { "x-api-key": $aa_key } https://artificialanalysis.ai/api/v2/language/models
        } catch { |e|
            http get -H { "x-api-key": $aa_key } https://artificialanalysis.ai/api/v2/language/models/free
        })

        if $res == null or ($res | get -o data | is-empty) {
            error make { msg: "Failed to fetch models." }
        }

        $mapped = ($res.data | each { |m|
            let pr = ($m | get -o pricing)
            let in_p = if $pr != null { ($pr | get -o price_1m_input_tokens | default 0.0) } else { 0.0 }
            let out_p = if $pr != null { ($pr | get -o price_1m_output_tokens | default 0.0) } else { 0.0 }
            $m | insert input $in_p | insert output $out_p
        })
    }

    # Cache to SQLite
    let payload = ($mapped | to json)
    if ($db_path | path exists) { rm -f $db_path }
    [{date: $today, payload: $payload}] | into sqlite $db_path -t cache

    $mapped
}

# Fuzzy match: exact id → substring id/name → fzf picker if multiple
export def find-model [query: string, models: list] {
    let m1 = ($models | where id == $query)
    if not ($m1 | is-empty) { return ($m1 | first) }

    let m2 = ($models | where { |m| ($m.id | str contains --ignore-case $query) or ($m.name | str contains --ignore-case $query) })
    if ($m2 | length) == 1 { return ($m2 | first) }
    if ($m2 | length) > 1 {
        # Multiple fuzzy matches → let user pick via fzf
        let lines = ($m2 | each { |m| $"($m.id) │ ($m.name) │ ($m.model_creator | get -o name | default '')" })
        let picked = ($lines | str join "\n" | fzf --reverse --prompt=$"Multiple matches for '($query)'▸ " --height=15)
        if ($picked | is-empty) { return null }
        let picked_id = ($picked | split row " │ " | first | str trim)
        return ($m2 | where id == $picked_id | first)
    }
    null
}
