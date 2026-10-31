$marketHost = switch ($env:KALSHI_ENV) {
    'production' { 'https://external-api.kalshi.com' }
    'demo' { 'https://external-api.demo.kalshi.co' }
    $null { 'https://external-api.demo.kalshi.co' }
    default { throw 'KALSHI_ENV must be demo or production.' }
}

$seriesTicker = 'KXBTC15M'
$cursor = $null
$markets = @(
    do {
        $uri = "$marketHost/trade-api/v2/markets?series_ticker=$seriesTicker&status=open&limit=1000"
        if ($cursor) {
            $uri += '&cursor=' + [Uri]::EscapeDataString($cursor)
        }

        $result = Invoke-RestMethod -Uri $uri -TimeoutSec 30 -ErrorAction Stop
        $result.markets
        $cursor = $result.cursor
    } while ($cursor)
)

# Pick the current available interval rather than a later listed BTC window.
$now = [DateTimeOffset]::UtcNow
$market = $markets |
    Where-Object {
        [DateTimeOffset]$_.open_time -le $now -and
        [DateTimeOffset]$_.close_time -gt $now
    } |
    Sort-Object { [DateTimeOffset]$_.close_time } |
    Select-Object -First 1

if (-not $market) {
    throw "No open BTC 15-minute market ($seriesTicker) found on $marketHost. For production markets, set KALSHI_ENV=production and run again."
}

$env:KALSHI_MARKET_TICKER = $market.ticker
$market | Select-Object ticker, title, close_time
