# Game dumps copied from Linux often turn symlinks (libstdc++.so.6 ->
# libstdc++.so.6.0.7, libboost_*.so.1.36.0 -> libboost_*.so, ...) into empty
# files, which LINE can't load. Copy the real library over each empty one.
# librt is left alone on purpose: LINE supplies its functions itself.
foreach ($dir in @('.', 'libso')) {
	if (-not (Test-Path $dir)) { continue }
	$files = @(Get-ChildItem -LiteralPath $dir -File -Filter '*.so*')
	foreach ($empty in $files | Where-Object { $_.Length -eq 0 -and $_.Name -notlike 'librt*' }) {
		$source = $files |
			Where-Object { $_.Length -gt 0 -and ($_.Name.StartsWith($empty.Name + '.') -or $empty.Name.StartsWith($_.Name + '.')) } |
			Sort-Object { $_.Name.Length } -Descending |
			Select-Object -First 1
		if ($source) {
			Copy-Item -LiteralPath $source.FullName -Destination $empty.FullName -Force
			Write-Output "[start] fixed $dir\$($empty.Name) (copied from $($source.Name))"
		}
	}
}
