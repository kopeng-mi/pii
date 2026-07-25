# pii.nu - Pi coding agent session analytics
# Source: source pii.nu
use llm-api.nu [compact-num, spark-bar]

def sessions-dir [] { $"($env.HOME)/.pi/agent/sessions/" }

# ── Pricing fallback from llm-stats.db ──────────────────────────────────────

def load-pricing-table [] {
    let db_path = ($nu.default-config-dir | path join "llm-stats.db")
    if not ($db_path | path exists) { return [] }
    try {
        let db = (open $db_path)
        if "cache" not-in ($db | columns) { return [] }
        let latest = ($db.cache | last | get payload | from json)
        if ($latest | is-empty) { [] } else { $latest }
    } catch { [] }
}

def match-price [model: string, pricing: list] {
    let clean = ($model | str replace --regex '^[^/]+/' '' | str replace '-thinking' '' | str replace --regex '-\d{8}$' '' | str replace --regex '-2\d{3}-\d{2}-\d{2}$' '')
    let dashed = ($clean | str replace --all '.' '-')

    let m1 = ($pricing | where id == $clean)
    if not ($m1 | is-empty) { return ($m1 | first) }

    let m2 = ($pricing | where id == $dashed)
    if not ($m2 | is-empty) { return ($m2 | first) }

    let m3 = ($pricing | where { |r| ($r.id | str starts-with $clean) or ($r.id | str starts-with $dashed) })
    if not ($m3 | is-empty) { return ($m3 | first) }

    null
}

def est-cost [in_tok: int, out_tok: int, price_rec] {
    if $price_rec == null { return 0.0 }
    let in_p = ($price_rec.input | default 0 | into float)
    let out_p = ($price_rec.output | default 0 | into float)
    if $in_p == 0.0 and $out_p == 0.0 { return 0.0 }
    (($in_tok | into float) * $in_p + ($out_tok | into float) * $out_p) / 1000000.0
}

# ── Cache ───────────────────────────────────────────────────────────────────

def cache-path [] { $"($env.HOME)/.pi/agent/pii-cache.nuon" }
def legacy-cache-path [] { $"($env.HOME)/.pi/agent/pii-cache.json" }

def load-cache [] {
    let p = (cache-path)
    if ($p | path exists) {
        try { open $p } catch { [] }
    } else {
        # migrate from JSON if it exists
        let lp = (legacy-cache-path)
        if ($lp | path exists) {
            let data = (try { open $lp } catch { [] })
            if not ($data | is-empty) { save-cache $data }
            $data
        } else { [] }
    }
}

def save-cache [cache: list] {
    $cache | to nuon | save -f (cache-path)
}

# Extract first user prompt from a JSONL file (truncated)
def extract-prompt [f: string] {
    # Only read the first 50 lines — prompt is always near the top
    let raw = (open $f --raw)
    let head_lines = ($raw | lines | first 50)
    for $line in $head_lines {
        let entry = (try { $line | from json } catch { {} })
        if ($entry | get -o type) == "message" and ($entry | get -o message.role) == "user" {
            let content = ($entry | get -o message.content | default [])
            let text = (try {
                let parts = ($content | where type == "text" | get -o text | default [])
                if ($parts | is-empty) { "" } else { $parts | first }
            } catch {
                try { $content | into string } catch { "" }
            })
            if ($text | is-empty) { continue }
            let words = ($text | str replace --all "\n" " " | split row " " | where { |w| $w != "" })
            let truncated = ($words | first 8 | str join " ")
            let suffix = (if ($words | length) > 8 { "..." } else { "" })
            return $"($truncated)($suffix)"
        }
    }
    ""
}

# Parse a single JSONL file into message rows
def parse-jsonl-file [f: string, project: string, pricing: list] {
    let fts = ($f | path basename | split row '_' | first)
    let file_date = ($fts | str substring 0..9)
    let prompt = (extract-prompt $f)
    mut rows = []
    for $line in (open $f --raw | lines) {
        let entry = (try { $line | from json } catch { {} })
        if ($entry | get -o type) == "message" and ($entry | get -o message.role) == "assistant" {
            let msg = $entry.message
            let usage = ($msg | get -o usage | default null)
            if $usage != null {
                let model_name = ($msg | get -o model | default "unknown")
                let in_tok = ($usage | get -o input | default 0)
                let out_tok = ($usage | get -o output | default 0)
                let pi_cost = ($usage | get -o cost.total | default 0 | into float)

                let final_cost = if $pi_cost > 0.0 {
                    $pi_cost
                } else if not ($pricing | is-empty) {
                    let price_rec = (match-price $model_name $pricing)
                    est-cost $in_tok $out_tok $price_rec
                } else { 0.0 }

                $rows = ($rows | append {
                    date:          $file_date
                    time:          ($fts | str substring 11..15 | str replace '-' ':')
                    session:       ($fts | str substring 0..18)
                    project:       $project
                    file:          $f
                    prompt:        $prompt
                    model:         $model_name
                    tokens:        ($usage | get -o totalTokens | default 0)
                    input_tokens:  $in_tok
                    output_tokens: $out_tok
                    cost:          $final_cost
                    error:         (($msg | get -o stopReason) == "error")
                })
            }
        }
    }
    $rows
}

# ── Data loading (cached) ──────────────────────────────────────────────────

def load-messages [
    --after: string
] {
    let base = (sessions-dir)
    if not ($base | path exists) { return [] }

    let pricing = (load-pricing-table)
    mut cache = (load-cache)
    mut msgs = []
    mut cache_dirty = false
    mut seen_paths = []

    let dirs = (ls $base | get name)
    for $dir in $dirs {
        let project = ($dir | path basename | str replace --all '-' '/' | str trim --left --char '/' | str trim --right --char '/' | split row '/' | last)
        let files = (ls $dir | where name =~ '.jsonl')
        for $fi in $files {
            let f = $fi.name
            let fts = ($f | path basename | split row '_' | first)
            let file_date = ($fts | str substring 0..9)
            if $after != null and $file_date < $after { continue }

            let norm_path = ($f | str replace --all '\\' '/')
            $seen_paths = ($seen_paths | append $norm_path)
            let fsize = ($fi.size | into int)
            let hit = ($cache | where path == $norm_path)

            if not ($hit | is-empty) and ($hit | first | get size) == $fsize {
                $msgs = ($msgs | append ($hit | first | get rows))
            } else {
                let rows = (parse-jsonl-file $f $project $pricing)
                $msgs = ($msgs | append $rows)
                $cache = ($cache | where path != $norm_path | append { path: $norm_path, size: $fsize, rows: $rows })
                $cache_dirty = true
            }
        }
    }

    # Prune stale cache entries only on full scans
    if $after == null {
        let before_len = ($cache | length)
        $cache = ($cache | where { |e| $e.path in $seen_paths })
        if ($cache | length) != $before_len { $cache_dirty = true }
    }

    if $cache_dirty { save-cache $cache }

    $msgs
}

# ── Grouping (carries session/file/prompt through) ─────────────────────────

def group-sessions [msgs] {
    $msgs | group-by session | items { |sid, rows|
        let first = ($rows | first)
        {
            date:    $first.date
            time:    $first.time
            session: $sid
            project: $first.project
            file:    $first.file
            prompt:  $first.prompt
            model:   ($rows | get model | uniq | str join ", ")
            calls:   ($rows | length)
            tokens:  ($rows | get tokens | math sum)
            cost:    ($rows | get cost | math sum | math round --precision 4)
            errors:  ($rows | where error | length)
        }
    }
}

# ── Formatting helpers ──────────────────────────────────────────────────────

# Value-inside progress bar, shared with llm-s / llm-c.
# Bright fill uses dark text; the unfilled remainder uses dim text.
def get-bar [val, max, width=16] {
    spark-bar $val $max $width -l (compact-num $val)
}

def format-cost [cost, max_cost] {
    if $cost <= 0.0 { return "\e[38;5;237m  --\e[0m" }
    let ratio = (if $max_cost > 0.0 { $cost / $max_cost } else { 0.0 })
    let color = (
        if $ratio > 0.75 { "\e[38;5;196m" }
        else if $ratio > 0.4 { "\e[38;5;214m" }
        else if $ratio > 0.15 { "\e[38;5;220m" }
        else { "\e[38;5;114m" }
    )
    $"($color)$($cost | math round --precision 4 | into string)\e[0m"
}

def short-text [text: string, width=26] {
    if ($text | str length) > $width {
        $"($text | str substring 0..($width - 2))…"
    } else { $text }
}

def format-model [model: string] {
    let color = (
        if ($model | str contains "claude") { "\e[38;5;215m" }
        else if ($model | str contains "gpt") { "\e[38;5;114m" }
        else if ($model | str contains "gemini") { "\e[38;5;75m" }
        else if ($model | str contains "deepseek") { "\e[38;5;147m" }
        else if ($model | str contains "kimi") { "\e[38;5;183m" }
        else if ($model | str contains "qwen") { "\e[38;5;180m" }
        else if ($model | str contains "glm") { "\e[38;5;109m" }
        else if ($model | str contains "mimo") { "\e[38;5;174m" }
        else if ($model | str contains "minimax") { "\e[38;5;168m" }
        else { "\e[38;5;250m" }
    )
    $"($color)($model)\e[0m"
}

def format-rows [rows] {
    let max_tokens = (if ($rows | is-empty) { 0 } else { $rows | get tokens | math max })
    let max_cost = (if ($rows | is-empty) { 0.0 } else { $rows | get cost | math max | into float })

    # Compact view: token count is embedded in the usage bar.
    $rows | each { |r| {
        when:    $"\e[38;5;242m($r.date | str substring 5..9) ($r.time)\e[0m"
        project: $"\e[1;38;5;255m(short-text $r.project 16)\e[0m"
        model:   (format-model (short-text $r.model 28))
        calls:   (if $r.calls > 10 { $"\e[38;5;220m($r.calls)\e[0m" } else if $r.calls > 5 { $"\e[38;5;250m($r.calls)\e[0m" } else { $"\e[38;5;242m($r.calls)\e[0m" })
        usage:   (get-bar $r.tokens $max_tokens 16)
        cost:    (format-cost $r.cost $max_cost)
        err:     (if $r.errors > 0 { $"\e[38;5;196m($r.errors)\e[0m" } else { "\e[38;5;237m·\e[0m" })
    }}
}

# ── Sorting ─────────────────────────────────────────────────────────────────

def sort-rows [rows, sort_by] {
    match $sort_by {
        "cost" => ($rows | sort-by cost --reverse),
        "tokens" => ($rows | sort-by tokens --reverse),
        "calls" => ($rows | sort-by calls --reverse),
        _ => ($rows | sort-by date time)
    }
}

# ── Section display ─────────────────────────────────────────────────────────

def show-rows [label, msgs, sort_by] {
    print ""
    if ($msgs | length) == 0 {
        print $"  \e[38;5;237m▰\e[0m \e[1m($label)\e[0m \e[38;5;237m· no sessions\e[0m"
        return
    }
    let rows = (sort-rows (group-sessions $msgs) $sort_by)

    let n = ($rows | length)
    let calls = ($rows | get calls | math sum)
    let tc = ($rows | get cost | math sum | math round --precision 2 | into string)

    print $"  \e[38;5;43m━━\e[0m \e[1;38;5;255m($label)\e[0m \e[38;5;246m· ($n) sessions · ($calls) calls ·\e[0m \e[38;5;220m$($tc)\e[0m"
    format-rows $rows | table -i false | print
}

# ── Heatmap ─────────────────────────────────────────────────────────────────

def show-heatmap [msgs] {
    let today = (date now)
    let today_wday = ($today | format date "%w" | into int)
    let cols = 26
    let days_back = ($cols * 7)

    let total_calls = ($msgs | length)
    let total_tokens = (if $total_calls == 0 { 0 } else { $msgs | get tokens | math sum })
    let total_tokens_display = (compact-num $total_tokens)
    let total_cost = (if $total_calls == 0 { 0.0 } else { $msgs | get cost | math sum | math round --precision 2 })

    let daily_stats = ($msgs | group-by date | items { |d, rows| { date: $d, tokens: ($rows | get tokens | math sum), cost: ($rows | get cost | math sum) } })
    let max_tokens = (if ($daily_stats | is-empty) { 1 } else { $daily_stats | get tokens | math max })

    print ""
    print $"  \e[38;5;43m━━\e[0m \e[1;38;5;255mActivity\e[0m \e[38;5;246m· ($days_back)d · ($total_calls) calls · ($total_tokens_display) tok ·\e[0m \e[38;5;220m$($total_cost)\e[0m"
    print ""

    mut top_row = []
    for c in 0..($cols - 1) { $top_row = ($top_row | append "  ") }
    mut prev_month = ""

    for c in 0..($cols - 1) {
        let offset = (($cols - 1 - $c) * 7) + $today_wday
        if $offset < 0 { continue }
        let d = $today - ($offset * 1day)
        let m = ($d | format date "%b")
        if $m != $prev_month {
            $top_row = ($top_row | update $c $m)
            $prev_month = $m
        }
    }

    mut header = "    "
    mut skip = 0
    for item in $top_row {
        if $skip > 0 { $skip = $skip - 1; continue }
        if ($item | str length) == 3 {
            $header = ($header + $item + " "); $skip = 1
        } else {
            $header = ($header + "  ")
        }
    }
    print $"\e[38;5;242m($header)\e[0m"

    let char = "■"
    for r in 0..6 {
        mut row_str = (match $r { 1 => "\e[38;5;242m  M \e[0m", 3 => "\e[38;5;242m  W \e[0m", 5 => "\e[38;5;242m  F \e[0m", _ => "    " })
        for c in 0..($cols - 1) {
            let offset = (($cols - 1 - $c) * 7) + ($today_wday - $r)
            if $offset < 0 {
                $row_str = ($row_str + "  ")
            } else {
                let d_str = ($today - ($offset * 1day) | format date "%Y-%m-%d")
                let stat = ($daily_stats | where date == $d_str)
                let t = (if ($stat | is-empty) { 0 } else { $stat.0.tokens })
                let color = (
                    if $t == 0 { "\e[38;5;237m" }
                    else if $t <= ($max_tokens * 0.10) { "\e[38;5;23m" }
                    else if $t <= ($max_tokens * 0.35) { "\e[38;5;30m" }
                    else if $t <= ($max_tokens * 0.70) { "\e[38;5;37m" }
                    else { "\e[38;5;43m" }
                )
                $row_str = ($row_str + $"($color)($char)\e[0m ")
            }
        }
        print $row_str
    }
    print ""
    print $"    \e[38;5;246mLess\e[0m \e[38;5;237m($char)\e[0m \e[38;5;23m($char)\e[0m \e[38;5;30m($char)\e[0m \e[38;5;37m($char)\e[0m \e[38;5;43m($char)\e[0m \e[38;5;246mMore\e[0m"
    print ""
}

# ── Session detail ──────────────────────────────────────────────────────────

def show-session-detail [session_id: string, msgs: list, --calls] {
    let rows = ($msgs | where session == $session_id)
    if ($rows | is-empty) {
        print $"  \e[38;5;237m▰\e[0m No data for session \e[38;5;242m($session_id)\e[0m"
        return
    }
    let first = ($rows | first)
    let total_tok = ($rows | get tokens | math sum)
    let total_in = ($rows | get input_tokens | math sum)
    let total_out = ($rows | get output_tokens | math sum)
    let total_cost = ($rows | get cost | math sum | math round --precision 4)
    let errs = ($rows | where error | length)

    print ""
    let prompt_str = ($first | get -o prompt | default "")
    let prompt_display = (if ($prompt_str | is-empty) { "" } else { $" \e[38;5;246m\"($prompt_str)\"\e[0m" })
    print $"  \e[38;5;43m▰\e[0m \e[1mSession\e[0m \e[38;5;242m($first.date) ($first.time)\e[0m · \e[1;38;5;255m($first.project)\e[0m($prompt_display)"
    let err_str = (if $errs > 0 { $"  \e[38;5;196merrors: ($errs)\e[0m" } else { "" })
    print $"    \e[38;5;246mcalls:\e[0m ($rows | length)  \e[38;5;246mtokens:\e[0m \e[38;5;43m(compact-num $total_tok)\e[0m  \e[38;5;246min:\e[0m (compact-num $total_in)  \e[38;5;246mout:\e[0m (compact-num $total_out)  \e[38;5;246mcost:\e[0m \e[38;5;220m$($total_cost)\e[0m($err_str)"
    print ""

    let by_model_raw = ($rows | group-by model | items { |m, rs| {
        model_name: $m
        calls: ($rs | length)
        tokens: ($rs | get tokens | math sum)
        "in": ($rs | get input_tokens | math sum)
        "out": ($rs | get output_tokens | math sum)
        cost: ($rs | get cost | math sum | math round --precision 4)
    }} | sort-by tokens --reverse)
    let max_model_tokens = ($by_model_raw | get tokens | math max)
    let max_model_cost = ($by_model_raw | get cost | math max | into float)
    let by_model = ($by_model_raw | each { |r| {
        model: (format-model (short-text $r.model_name 28))
        calls: $r.calls
        usage: (get-bar $r.tokens $max_model_tokens 16)
        "in": (compact-num $r.in)
        "out": (compact-num $r.out)
        cost: (format-cost $r.cost $max_model_cost)
    }})
    print $"  \e[38;5;37m  Models\e[0m"
    $by_model | table -i false | print

    if not $calls {
        print $"\n    \e[38;5;237mUse \e[0m\e[38;5;37mpii -i --calls\e[0m\e[38;5;237m for the full call timeline.\e[0m"
        return
    }

    let max_t = ($rows | get tokens | math max)
    let max_c = ($rows | get cost | math max | into float)
    let timeline = ($rows | enumerate | each { |it| {
        "#":     ($it.index + 1)
        model:   (format-model (short-text $it.item.model 28))
        "in":    (compact-num $it.item.input_tokens)
        "out":   (compact-num $it.item.output_tokens)
        usage:   (get-bar $it.item.tokens $max_t 16)
        cost:    (format-cost $it.item.cost $max_c)
        err:     (if $it.item.error { "\e[38;5;196m✗\e[0m" } else { "\e[38;5;237m·\e[0m" })
    }})
    print ""
    print $"  \e[38;5;37m  Calls\e[0m"
    $timeline | table -i false | print
}

# ── Interactive session picker (fzf) ───────────────────────────────────────
# Optimized: uses pre-grouped data (session/file/prompt already in rows)
# so no O(n*m) re-scan of all messages.

def pick-session [grouped: list, prompt: string, mode: string] {
    if ($grouped | is-empty) {
        print $"  \e[38;5;237m▰\e[0m No sessions found"
        return null
    }

    # Build fzf lines directly from grouped data — no re-scanning msgs
    let lines = ($grouped | each { |r|
        let p = ($r | get -o prompt | default "")
        let line_text = if $mode == "continue" {
            $"($r.date) ($r.time) │ ($r.project) │ ($p)"
        } else {
            $"($r.date) ($r.time) │ ($r.project) │ ($p) │ ($r.model) │ ($r.calls) calls │ ($r.tokens) tok │ $($r.cost)"
        }
        {
            line: $line_text
            session: $r.session
            file: $r.file
        }
    })

    let fzf_input = ($lines | get line | str join "\n")
    let picked = (try {
        $fzf_input | fzf --ansi --prompt=$prompt --height=20 --reverse --info=inline --no-multi | str trim
    } catch { "" })

    if ($picked | is-empty) { return null }

    let idx = ($lines | enumerate | where { |it| $it.item.line == $picked })
    if ($idx | is-empty) { return null }
    let match = ($idx | first | get item)
    { session: $match.session, file: $match.file }
}

# ── Main command ────────────────────────────────────────────────────────────

export def pii [
    --today (-t)          # Today's sessions
    --week (-w)           # Past 7 days
    --month (-m)          # Past 30 days
    --heatmap (-H)        # 180-day activity heatmap
    --summary (-s)        # Summary dashboard
    --inspect (-i)        # Fuzzy-pick a session and show detail
    --continue (-c)       # Fuzzy-pick a session and continue in pi
    --calls                # With -i: show every call in the selected session
    --query (-q): string  # Filter by model
    --sort: string = "time"  # Sort: time | cost | tokens | calls
    --days (-d): int      # Limit picker to last N days (default 90 for -i/-c)
] {
    if not $today and not $week and not $month and not $heatmap and not $summary and not $inspect and not $continue {
        print ""
        print $"  \e[38;5;43m▰\e[0m \e[1mpii\e[0m \e[38;5;246m· session analytics\e[0m"
        print ""
        print $"    \e[38;5;37m-t\e[0m  \e[38;5;246mtoday\e[0m        \e[38;5;37m-w\e[0m  \e[38;5;246mweek\e[0m       \e[38;5;37m-m\e[0m  \e[38;5;246mmonth\e[0m"
        print $"    \e[38;5;37m-H\e[0m  \e[38;5;246mheatmap\e[0m      \e[38;5;37m-s\e[0m  \e[38;5;246msummary\e[0m    \e[38;5;37m-q\e[0m  \e[38;5;246mfilter model\e[0m"
        print $"    \e[38;5;37m-i\e[0m  \e[38;5;246minspect\e[0m      \e[38;5;37m-c\e[0m  \e[38;5;246mcontinue\e[0m   \e[38;5;37m--calls\e[0m  \e[38;5;246mfull inspection timeline\e[0m"
        print $"    \e[38;5;37m-d\e[0m  \e[38;5;246mdays\e[0m         \e[38;5;246mscope picker to last N days \(default 90\)\e[0m"
        print ""
        print $"    \e[38;5;237mcost = pi reported, fallback to llm-stats.db pricing\e[0m"
        print ""
        return
    }

    let today_date = (date now | format date "%Y-%m-%d")
    let week_ago = ((date now) - 7day | format date "%Y-%m-%d")
    let month_ago = ((date now) - 30day | format date "%Y-%m-%d")
    let heatmap_ago = ((date now) - 182day | format date "%Y-%m-%d")

    # For -i/-c: default to 90 days, customizable with -d
    let picker_days = ($days | default 90)
    let picker_ago = ((date now) - ($picker_days * 1day) | format date "%Y-%m-%d")

    let cutoff = (
        if $inspect or $continue { $picker_ago }
        else if $summary or $month { null }
        else if $heatmap { $heatmap_ago }
        else if $week { $week_ago }
        else if $today { $today_date }
        else { null }
    )

    let msgs = (load-messages --after $cutoff)
    let filtered = (if $query != null { $msgs | where { |m| $m.model | str contains --ignore-case $query } } else { $msgs })

    if $heatmap { show-heatmap $filtered }

    if $today { show-rows $"Today [($today_date)]" ($filtered | where date == $today_date) $sort }
    if $week { show-rows "Past 7 Days" ($filtered | where date >= $week_ago) $sort }
    if $month { show-rows "Past 30 Days" ($filtered | where date >= $month_ago) $sort }

    if $summary {
        def agg [msgs, label] {
            let cv = (if ($msgs | is-empty) { 0.0 } else { $msgs | get cost | math sum | math round --precision 2 })
            let errs = (if ($msgs | is-empty) { 0 } else { $msgs | where error | length })
            let toks = (if ($msgs | is-empty) { 0 } else { $msgs | get tokens | math sum })
            let in_t = (if ($msgs | is-empty) { 0 } else { $msgs | get input_tokens | math sum })
            let out_t = (if ($msgs | is-empty) { 0 } else { $msgs | get output_tokens | math sum })
            let sess = (if ($msgs | is-empty) { 0 } else { $msgs | get session | uniq | length })

            {
                period:   $"\e[1m($label)\e[0m",
                sessions: $"\e[38;5;246m($sess)\e[0m",
                calls:    $"\e[38;5;246m($msgs | length)\e[0m",
                tokens:   $"\e[38;5;43m(compact-num $toks)\e[0m",
                "in":     $"\e[38;5;242m(compact-num $in_t)\e[0m",
                "out":    $"\e[38;5;242m(compact-num $out_t)\e[0m",
                cost:     (if $cv > 0 { $"\e[38;5;220m$($cv)\e[0m" } else { "\e[38;5;237m--\e[0m" }),
                errors:   (if $errs > 0 { $"\e[38;5;196m($errs)\e[0m" } else { "\e[38;5;237m-\e[0m" })
            }
        }
        print ""
        print $"  \e[38;5;43m━━\e[0m \e[1;38;5;255mSummary\e[0m"
        [
          (agg ($filtered | where date == $today_date) "Today")
          (agg ($filtered | where date >= $week_ago) "7 Days")
          (agg ($filtered | where date >= $month_ago) "30 Days")
          (agg $filtered "All Time")
        ] | table -i false | print
    }

    if $inspect {
        let grouped = (group-sessions $filtered | sort-by date time --reverse)
        let pick = (pick-session $grouped "inspect▸ " "inspect")
        if $pick != null {
            show-session-detail $pick.session $filtered --calls=$calls
        }
    }

    if $continue {
        let grouped = (group-sessions $filtered | sort-by date time --reverse)
        let pick = (pick-session $grouped "continue▸ " "continue")
        if $pick != null {
            let session_file = $pick.file
            print $"  \e[38;5;43m▰\e[0m Resuming session \e[38;5;246m($session_file | path basename)\e[0m"
            pi --session $session_file
        }
    }
}
