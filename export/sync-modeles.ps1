# Copie les modèles de `modeles\` dans `src-tauri\templates\`, embarqué par Tauri. Miroir exact :
# un modèle retiré du dossier disparaît de l'application à la construction suivante.
$racine = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
$source = Join-Path $racine "modeles"
$cible = Join-Path $racine "src-tauri\templates"
New-Item -ItemType Directory -Force -Path $cible | Out-Null
Get-ChildItem -Path $cible -File | Where-Object { $_.Extension -in ".graf", ".txt" } | Remove-Item -Force
$n = 0
if (Test-Path $source) {
  foreach ($f in Get-ChildItem -Path $source -Filter "*.graf" -File) {
    Copy-Item $f.FullName $cible -Force
    $txt = [System.IO.Path]::ChangeExtension($f.FullName, ".txt")
    if (Test-Path $txt) { Copy-Item $txt $cible -Force }
    $n++
  }
}
Write-Host "  Modèles embarqués : $n"
