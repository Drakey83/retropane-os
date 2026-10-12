# Makes Windows find the Axim again (ADR-0012). The X51v's D+ pull-up is a
# fixed resistor, so Windows never sees the device leave and return: after
# a RetroPane image replaces Windows Mobile (or the loader restarts), the
# port still holds a stale entry, Windows Mobile's own device or ours.
# Restarting every present entry that could be the Axim makes Windows
# enumerate the port afresh. Needs admin rights: run by the scheduled task
# `cargo xtask load --setup` creates (or elevated by hand).
$ProgressPreference = 'SilentlyContinue'
$ids = (Get-PnpDevice | Where-Object {
    $_.Present -and $_.InstanceId -match '^USB\\VID_(1209|413C|0000)&'
}).InstanceId
foreach ($id in $ids) {
    pnputil /restart-device "$id" | Out-Null
}
