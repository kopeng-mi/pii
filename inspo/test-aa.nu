let key = $env.ARTIFICIALANALYSIS_API_KEY? | default ""
if $key == "" { print "no key" } else {
    try {
        let res = (http get -H [x-api-key $key] https://artificialanalysis.ai/api/v2/language/models/free)
        $res.data | first 1 | to json
    } catch { |e| print $e }
}
