$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$tempDir = Get-ChildItem -LiteralPath ([IO.Path]::GetTempPath()) -Directory -Filter 'aitierlist-retry-*' |
    Sort-Object LastWriteTime -Descending |
    Where-Object { Test-Path -LiteralPath (Join-Path $_.FullName 'leaderboard-live.json') } |
    Select-Object -First 1 -ExpandProperty FullName
if (!$tempDir) {
    $tempDir = Join-Path ([IO.Path]::GetTempPath()) ('aitierlist-retry-' + [guid]::NewGuid())
    New-Item -ItemType Directory -Path $tempDir | Out-Null
}
$destination = Join-Path $PSScriptRoot '../assets/retry-correlation.json'
$existingAsset = if (Test-Path -LiteralPath $destination) {
    Get-Content -LiteralPath $destination -Raw | ConvertFrom-Json -AsHashtable
} else {
    $null
}
$deepUrl = 'https://deepswe.datacurve.ai/artifacts/v1.1'
foreach ($name in @('leaderboard-live', 'trials', 'tasks')) {
    if (!(Test-Path -LiteralPath (Join-Path $tempDir "$name.json"))) {
        Invoke-WebRequest "$deepUrl/$name.json" -OutFile (Join-Path $tempDir "$name.json")
    }
}
$leaderboard = Get-Content (Join-Path $tempDir 'leaderboard-live.json') -Raw | ConvertFrom-Json -AsHashtable
$trials = Get-Content (Join-Path $tempDir 'trials.json') -Raw | ConvertFrom-Json -AsHashtable
$tasks = Get-Content (Join-Path $tempDir 'tasks.json') -Raw | ConvertFrom-Json -AsHashtable
$cells = @{}
foreach ($config in $leaderboard.rows) {
    $cells[$config.config] = @{}
}
foreach ($trial in $trials.rows) {
    if ($trial.source -ne 'deep-swe' -or !$trial.included_in_score -or !$cells.ContainsKey($trial.config)) {
        continue
    }
    $configCells = $cells[$trial.config]
    if (!$configCells.ContainsKey($trial.task_name)) {
        $configCells[$trial.task_name] = [double[]]@(0, 0)
    }
    $configCells[$trial.task_name][0] += [int][bool]$trial.passed
    $configCells[$trial.task_name][1] += 1
}
function Limit-Rho([double]$value) {
    [Math]::Clamp($value, 0.0, [Math]::BitDecrement(1.0))
}
function Get-Pool($moments) {
    $numerator = 0.0
    $denominator = 0.0
    foreach ($moment in $moments) {
        $numerator += ($moment.count - 1) * $moment.tau
        $denominator += ($moment.count - 1) * $moment.d
    }
    Limit-Rho ($numerator / $denominator)
}
$difficulty = @{}
foreach ($task in $tasks.rows) {
    $sum = 0.0
    $count = 0
    foreach ($config in $leaderboard.rows) {
        $cell = $cells[$config.config][$task.id]
        if ($null -ne $cell) {
            $sum += $cell[0] / $cell[1]
            $count += 1
        }
    }
    $difficulty[$task.id] = $sum / $count
}
function Get-Moment($samples, [Random]$random) {
    $count = $samples.Count
    $sum = 0.0
    $squares = 0.0
    $inverseTrials = 0.0
    $weighted = 0.0
    $weights = 0.0
    for ($index = 0; $index -lt $count; $index++) {
        $sample = if ($null -eq $random) { $samples[$index] } else { $samples[$random.Next($count)] }
        $value = $sample.q
        $sum += $value
        $squares += $value * $value
        $inverseTrials += $sample.a
        $weighted += $sample.weight * $value
        $weights += $sample.weight
    }
    $mean = $sum / $count
    $variance = [Math]::Max(0.0, ($squares - $sum * $mean) / ($count - 1))
    $a = $inverseTrials / $count
    $d = $mean * (1.0 - $mean)
    $tau = ($variance - $a * $d) / (1.0 - $a)
    @{
        count = $count
        tau = $tau
        d = $d
        rho = if ($d -gt 0.0) { Limit-Rho ($tau / $d) } else { $null }
        ratio = if ($mean -gt 0.0) { $weighted / $weights / $mean } else { $null }
    }
}
function Get-Variance($values) {
    $values = @($values | Where-Object { $null -ne $_ })
    $mean = ($values | Measure-Object -Average).Average
    $squares = 0.0
    foreach ($value in $values) {
        $squares += ($value - $mean) * ($value - $mean)
    }
    $squares / ($values.Count - 1)
}
function Get-Shrunk([double]$value, [double]$variance, [double]$prior, [double]$priorVariance) {
    if ($variance -eq 0.0 -and $priorVariance -eq 0.0) {
        return $prior
    }
    ($value * $priorVariance + $prior * $variance) / ($variance + $priorVariance)
}
$bootstrapDraws = 400
$bootstrapSeed = 113400
$random = [Random]::new($bootstrapSeed)
$moments = @{}
$bootstraps = @{}
foreach ($config in ($leaderboard.rows | Sort-Object config)) {
    $configCells = $cells[$config.config]
    $samples = @($configCells.Keys | Sort-Object | ForEach-Object {
        $cell = $configCells[$_]
        @{ q = $cell[0] / $cell[1]; a = 1.0 / $cell[1]; weight = 1.0 - $difficulty[$_] }
    })
    $moment = Get-Moment $samples $null
    $draws = @(for ($draw = 0; $draw -lt $bootstrapDraws; $draw++) {
        Get-Moment $samples $random
    })
    $moment.rho_variance = Get-Variance $draws.rho
    $moment.ratio_variance = Get-Variance $draws.ratio
    $moment.rho_valid_draws = @($draws.rho | Where-Object { $null -ne $_ }).Count
    $moment.ratio_valid_draws = @($draws.ratio | Where-Object { $null -ne $_ }).Count
    $moments[$config.config] = $moment
    $bootstraps[$config.config] = $draws
}
$models = @($leaderboard.rows | Group-Object model | Sort-Object Name | ForEach-Object {
    $group = $_
    $poolMoments = @($group.Group | ForEach-Object { $moments[$_.config] })
    $poolRho = Get-Pool $poolMoments
    $poolRatio = ($poolMoments.ratio | Measure-Object -Average).Average
    $poolDraws = @(for ($draw = 0; $draw -lt $bootstrapDraws; $draw++) {
        $drawMoments = @($group.Group | ForEach-Object { $bootstraps[$_.config][$draw] })
        @{
            rho = if (($drawMoments.d | Measure-Object -Sum).Sum -gt 0.0) { Get-Pool $drawMoments } else { $null }
            ratio = if (@($drawMoments | Where-Object { $null -eq $_.ratio }).Count -eq 0) {
                ($drawMoments.ratio | Measure-Object -Average).Average
            } else { $null }
        }
    })
    $poolRhoVariance = Get-Variance $poolDraws.rho
    $poolRatioVariance = Get-Variance $poolDraws.ratio
    [ordered]@{
        model = $group.Name
        rho = $poolRho
        rho_variance = $poolRhoVariance
        rho_valid_draws = @($poolDraws.rho | Where-Object { $null -ne $_ }).Count
        difficulty_ratio = $poolRatio
        difficulty_ratio_variance = $poolRatioVariance
        difficulty_ratio_valid_draws = @($poolDraws.ratio | Where-Object { $null -ne $_ }).Count
        configs = @($group.Group | Sort-Object reasoning_effort | ForEach-Object {
            $moment = $moments[$_.config]
            [ordered]@{
                reasoning_effort = $_.reasoning_effort
                rho_raw = $moment.rho
                rho_shrunk = Get-Shrunk $moment.rho $moment.rho_variance $poolRho $poolRhoVariance
                rho_variance = $moment.rho_variance
                rho_valid_draws = $moment.rho_valid_draws
                difficulty_ratio_raw = $moment.ratio
                difficulty_ratio_shrunk = Get-Shrunk $moment.ratio $moment.ratio_variance $poolRatio $poolRatioVariance
                difficulty_ratio_variance = $moment.ratio_variance
                difficulty_ratio_valid_draws = $moment.ratio_valid_draws
            }
        })
    }
})
$meanRatio = ($models.configs.difficulty_ratio_shrunk | Measure-Object -Average).Average
foreach ($config in $models.configs) {
    $config.difficulty_ratio = $config.difficulty_ratio_shrunk / $meanRatio
}
$repoUrl = 'https://api.github.com/repos/harbor-framework/terminal-bench-2-1'
$revision = if ($existingAsset) { $existingAsset.sources[1].revision_or_generated_at } else { (Invoke-RestMethod "$repoUrl/commits/main").sha }
$files = Invoke-RestMethod "$repoUrl/contents/leaderboard/submissions?ref=$revision"
$submissions = @($files | Where-Object name -Like '*.json' | Sort-Object name | ForEach-Object {
    $path = Join-Path $tempDir $_.name
    if (!(Test-Path -LiteralPath $path)) {
        Invoke-WebRequest $_.download_url -OutFile $path
    }
    $submission = Get-Content $path -Raw | ConvertFrom-Json -AsHashtable
    $metrics = $submission.metrics
    @{
        date = $submission.metadata.date
        agent_display = $submission.metadata.agent_display.label
        model_display = $submission.metadata.model_display.label
        reasoning_effort = $submission.metadata.reasoning_effort
        accuracy = $metrics.accuracy
        n_trials = $metrics.n_trials
        observed = [double[]]@(($metrics.accuracy / 100.0), $metrics.pass_at_2, $metrics.pass_at_3, $metrics.pass_at_4, $metrics.pass_at_5)
    }
})
function Get-Loss([double]$rho, $entries) {
    $loss = 0.0
    $ratio = $rho / (1.0 - $rho)
    foreach ($entry in $entries) {
        $failure = 1.0
        for ($j = 0; $j -lt 5; $j++) {
            $failure *= 1.0 - $entry.observed[0] / (1.0 + $j * $ratio)
            $errorValue = 1.0 - $failure - $entry.observed[$j]
            $loss += $errorValue * $errorValue
        }
    }
    $loss
}
function Fit-Rho($entries) {
    $left = 0.0
    $right = [Math]::BitDecrement(1.0)
    for ($iteration = 0; $iteration -lt 90; $iteration++) {
        $a = $left + ($right - $left) / 3.0
        $b = $right - ($right - $left) / 3.0
        if ((Get-Loss $a $entries) -le (Get-Loss $b $entries)) {
            $right = $b
        } else {
            $left = $a
        }
    }
    $best = ($left + $right) / 2.0
    foreach ($endpoint in @(0.0, [Math]::BitDecrement(1.0))) {
        if ((Get-Loss $endpoint $entries) -lt (Get-Loss $best $entries)) {
            $best = $endpoint
        }
    }
    $best
}
$harnesses = @($submissions | Group-Object agent_display | Sort-Object Name | ForEach-Object {
    [ordered]@{
        harness_label = $_.Name
        rho = Fit-Rho $_.Group
        submissions = @($_.Group | Sort-Object date -Descending | ForEach-Object {
            [ordered]@{
                agent_display = $_.agent_display
                model_display = $_.model_display
                reasoning_effort = $_.reasoning_effort
                rho = Fit-Rho @($_)
                accuracy = $_.accuracy
                n_trials = $_.n_trials
            }
        })
    }
})
$terminalBenchV21 = [ordered]@{ pool_rho = Fit-Rho $submissions; harnesses = $harnesses }
$terminalBenchV21Revision = $revision
$terminalBenchV4Revision = '83c7a6172d629c6575b785ab12c8db787bb2e323'
$terminalBenchV4Repo = 'https://api.github.com/repos/harbor-framework/terminal-bench'
$files = Invoke-RestMethod "$terminalBenchV4Repo/contents/leaderboard/submissions?ref=$terminalBenchV4Revision"
$submissions = @($files | Where-Object name -Like '*.json' | Sort-Object name | ForEach-Object {
    $submission = Invoke-RestMethod $_.download_url
    $metrics = $submission.metrics
    @{
        date = $submission.metadata.date
        agent_display = $submission.metadata.agent_display.label
        model_display = $submission.metadata.model_display.label
        reasoning_effort = $submission.metadata.reasoning_effort
        accuracy = $metrics.accuracy
        n_trials = $metrics.n_trials
        observed = [double[]]@(($metrics.accuracy / 100.0), $metrics.pass_at_2, $metrics.pass_at_3, $metrics.pass_at_4, $metrics.pass_at_5)
    }
})
$harnesses = @($submissions | Group-Object agent_display | Sort-Object Name | ForEach-Object {
    [ordered]@{
        harness_label = $_.Name
        rho = Fit-Rho $_.Group
        submissions = @($_.Group | Sort-Object date -Descending | ForEach-Object {
            [ordered]@{
                agent_display = $_.agent_display
                model_display = $_.model_display
                reasoning_effort = $_.reasoning_effort
                rho = Fit-Rho @($_)
                accuracy = $_.accuracy
                n_trials = $_.n_trials
            }
        })
    }
})
$terminalBenchV4 = [ordered]@{ pool_rho = Fit-Rho $submissions; harnesses = $harnesses }
$result = [ordered]@{
    generated_at = [DateTimeOffset]::UtcNow.ToString('o')
    sources = @(
        [ordered]@{ name = 'DeepSWE v1.1'; url = $deepUrl; revision_or_generated_at = $leaderboard.generated_at }
        [ordered]@{ name = 'Terminal-Bench 2.1'; url = 'https://github.com/harbor-framework/terminal-bench-2-1/tree/main/leaderboard/submissions'; revision_or_generated_at = $terminalBenchV21Revision }
        [ordered]@{ name = 'Terminal-Bench 4.0'; url = 'https://github.com/harbor-framework/terminal-bench/tree/main/leaderboard/submissions'; revision_or_generated_at = $terminalBenchV4Revision }
    )
    deepswe = [ordered]@{ pool_rho = Get-Pool $moments.Values; bootstrap_draws = $bootstrapDraws; bootstrap_seed = $bootstrapSeed; difficulty_ratio_normalizer = $meanRatio; models = $models }
    terminal_bench = $terminalBenchV21
    terminal_bench_v4 = $terminalBenchV4
}
$json = ($result | ConvertTo-Json -Depth 12).Replace("`r`n", "`n")
[IO.File]::WriteAllText($destination, $json + "`n", [Text.UTF8Encoding]::new($false))
