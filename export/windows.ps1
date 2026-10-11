# Construit Lumitype pour Windows et range les fichiers dans export\sorties\ :
#   - Lumitype_x.y.z_x64-setup.exe : l'installateur à distribuer
#   - Lumitype_x.y.z_x64_en-US.msi : installateur MSI (déploiement en entreprise)
#   - Lumitype-portable.exe        : l'exécutable seul, sans installation
#
# Au premier lancement, ce qui manque est installé automatiquement, avec winget s'il est présent, sinon en
# téléchargeant directement les installateurs officiels (Microsoft, rust-lang.org, nodejs.org) :
#   - les Build Tools de Visual Studio (compilateur C++ de Microsoft, nécessaire à Rust sous Windows) ;
#   - Rust (via rustup) ;
#   - Node.js (version LTS) ;
#   - WebView2, s'il manquait (déjà présent sur un Windows à jour).
# Windows demandera l'autorisation d'installer (fenêtre de contrôle de compte) : acceptez.
#
# Lancez plutôt export\windows.bat (double-clic) : il appelle ce script sans buter sur la stratégie
# d'exécution de PowerShell, et depuis n'importe quel dossier — le script se place tout seul dans celui
# du projet. Options : 0.3.7 (fixer d'abord le numéro de version), -Reinstaller (refaire les dépendances
# de l'interface de zéro), -SansInstallation (ne rien installer, seulement vérifier), -Essai (compiler sans
# installateur puis lancer Lumitype, pour essayer).
#
# Moteur d'affichage : WebView2, le Chromium fourni par Windows (sous Linux, Lumitype embarque le sien). L'essai d'un
# Chromium embarqué sous Windows a été abandonné (fenêtre blanche, runtime jamais éprouvé sous Windows) ; le code
# Linux (export/linux.sh, crates/tauri-runtime-cef) n'est pas concerné.
param(
  [Parameter(Position = 0)][string]$Version = "",
  [switch]$SansInstallation,
  [switch]$Reinstaller,
  [switch]$Essai
)
$ErrorActionPreference = "Stop"
# Accents corrects dans la console, quelle que soit sa page de codes.
try { [Console]::OutputEncoding = [System.Text.Encoding]::UTF8 } catch {}
Set-Location (Join-Path $PSScriptRoot "..")
$racine = Get-Location
$sorties = Join-Path $racine "export\sorties"

function Titre($texte) { Write-Host "`n== $texte" -ForegroundColor White }
function Erreur($texte) { Write-Host $texte -ForegroundColor Red }
function Ok($texte) { Write-Host "  $texte" -ForegroundColor Green }

# Relit le PATH de Windows : ce qu'une installation vient d'ajouter devient utilisable sans rouvrir la console.
# Le PATH d'origine est conservé, ainsi que le dossier des applications du Store (où vit winget).
$pathOrigine = $env:Path
function Rafraichir-Path {
  $machine = [Environment]::GetEnvironmentVariable("Path", "Machine")
  $user = [Environment]::GetEnvironmentVariable("Path", "User")
  $cargo = Join-Path $env:USERPROFILE ".cargo\bin"
  $store = Join-Path $env:LOCALAPPDATA "Microsoft\WindowsApps"
  $node = Join-Path $env:ProgramFiles "nodejs"
  $env:Path = "$machine;$user;$cargo;$store;$node;$pathOrigine"
}

# Dossier des installateurs téléchargés.
$telechargements = Join-Path $env:TEMP "lumitype-outils"
New-Item -ItemType Directory -Force -Path $telechargements | Out-Null
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

function Telecharger($url, $nom) {
  $cible = Join-Path $telechargements $nom
  Write-Host "  Téléchargement de $nom..." -ForegroundColor Yellow
  $ProgressPreference = "SilentlyContinue" # sans la barre de PowerShell, le téléchargement est bien plus rapide
  Invoke-WebRequest -Uri $url -OutFile $cible -UseBasicParsing
  return $cible
}

# Lance un installateur et attend sa fin ; Windows demande l'autorisation si besoin.
function Executer($programme, $arguments) {
  $processus = Start-Process -FilePath $programme -ArgumentList $arguments -Wait -PassThru
  # 3010 = réussi, redémarrage conseillé.
  if ($processus.ExitCode -ne 0 -and $processus.ExitCode -ne 3010) {
    Erreur "L'installation a échoué (code $($processus.ExitCode))."
    exit 1
  }
  if ($processus.ExitCode -eq 3010) { Write-Host "  Installation terminée ; un redémarrage de Windows sera conseillé à la fin." -ForegroundColor Yellow }
}

# Installation SANS winget : les installateurs officiels, téléchargés et lancés directement.
function Installer-Directement($identifiant) {
  switch ($identifiant) {
    "Microsoft.VisualStudio.2022.BuildTools" {
      $fichier = Telecharger "https://aka.ms/vs/17/release/vs_BuildTools.exe" "vs_BuildTools.exe"
      Executer $fichier "--wait --passive --norestart --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
    }
    "Rustlang.Rustup" {
      $fichier = Telecharger "https://win.rustup.rs/x86_64" "rustup-init.exe"
      Executer $fichier "-y --default-toolchain stable --profile minimal"
    }
    "OpenJS.NodeJS.LTS" {
      # Dernière version LTS, lue dans l'index officiel de nodejs.org.
      $index = Invoke-RestMethod -Uri "https://nodejs.org/dist/index.json" -UseBasicParsing
      $lts = $index | Where-Object { $_.lts } | Select-Object -First 1
      $nom = "node-$($lts.version)-x64.msi"
      $fichier = Telecharger "https://nodejs.org/dist/$($lts.version)/$nom" $nom
      Executer "msiexec.exe" "/i `"$fichier`" /passive /norestart"
    }
    "Microsoft.EdgeWebView2Runtime" {
      $fichier = Telecharger "https://go.microsoft.com/fwlink/p/?LinkId=2124703" "MicrosoftEdgeWebview2Setup.exe"
      Executer $fichier "/silent /install"
    }
  }
}

function Present($commande) { return [bool](Get-Command $commande -ErrorAction SilentlyContinue) }

# Version 'majeure.mineure' d'un outil, lue dans sa sortie ('v22.11.0', 'rustc 1.91.1 (...)').
function Version-De($commande, $argument) {
  try {
    $texte = & $commande $argument 2>$null | Select-Object -First 1
    if ($texte -match '(\d+)\.(\d+)') { return [version]"$($Matches[1]).$($Matches[2])" }
  } catch {}
  return $null
}

# Les Build Tools (compilateur C++ de Microsoft) sont repérés par vswhere, l'outil officiel de Visual Studio.
function BuildTools-Presents {
  $vswhere = Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\Installer\vswhere.exe"
  if (-not (Test-Path $vswhere)) { return $false }
  $chemin = & $vswhere -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath 2>$null
  return [bool]$chemin
}

function WebView2-Present {
  $cles = @(
    "HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}",
    "HKLM:\SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}",
    "HKCU:\SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}"
  )
  foreach ($cle in $cles) {
    $version = (Get-ItemProperty -Path $cle -Name pv -ErrorAction SilentlyContinue).pv
    if ($version -and $version -ne "0.0.0.0") { return $true }
  }
  return $false
}

function Installer($nom, $identifiant, $arguments = @()) {
  if ($SansInstallation) {
    Erreur "Manquant : $nom (relancez sans -SansInstallation pour l'installer automatiquement)"
    exit 1
  }
  if (-not (Present "winget")) {
    # Pas de winget (Windows ancien, LTSC, ou Store absent) : les installateurs officiels sont téléchargés directement.
    Write-Host "  Installation de $nom (téléchargement direct, winget n'étant pas disponible)..." -ForegroundColor Yellow
    Installer-Directement $identifiant
    Rafraichir-Path
    return
  }
  Write-Host "  Installation de $nom..." -ForegroundColor Yellow
  $base = @("install", "--id", $identifiant, "--exact", "--source", "winget", "--accept-package-agreements", "--accept-source-agreements", "--disable-interactivity")
  & winget @base @arguments
  # winget répond 0 si l'installation a réussi, et un code 'déjà installé' si rien n'était à faire.
  $dejaInstalle = -1978335189
  if ($LASTEXITCODE -ne 0 -and $LASTEXITCODE -ne $dejaInstalle) {
    Erreur "L'installation de $nom a échoué (code $LASTEXITCODE)."
    exit 1
  }
  Rafraichir-Path
}

# ---------------------------------------------------------------- outils
Titre "Vérification des outils (installation automatique de ce qui manque)"
Rafraichir-Path

# 1. Build Tools de Visual Studio : à installer AVANT Rust, qui s'en sert pour lier les programmes.
if (BuildTools-Presents) {
  Ok "Build Tools C++ de Visual Studio : présents"
} else {
  Write-Host "  Build Tools C++ : absents. Téléchargement d'environ 3 Go, 10 à 20 minutes." -ForegroundColor Yellow
  Installer "Build Tools de Visual Studio 2022" "Microsoft.VisualStudio.2022.BuildTools" @("--override", "--wait --passive --norestart --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended")
  if (-not (BuildTools-Presents)) {
    Erreur "Les Build Tools semblent installés sans le compilateur C++."
    Write-Host "  Ouvrez 'Visual Studio Installer', Modifier, cochez 'Développement Desktop en C++', puis relancez."
    exit 1
  }
  Ok "Build Tools C++ de Visual Studio : installés"
}

# 2. Rust : rustup, puis la chaîne stable ; le projet demande Rust 1.88 ou plus récent.
if (-not (Present "rustup")) { Installer "Rust (rustup)" "Rustlang.Rustup" }
if (-not (Present "rustup")) { Erreur "rustup reste introuvable après l'installation : rouvrez la console et relancez."; exit 1 }
if (-not (Present "cargo")) {
  & rustup default stable
  Rafraichir-Path
}
$rust = Version-De "rustc" "--version"
if (-not $rust -or $rust -lt [version]"1.88") {
  Write-Host "  Mise à jour de Rust..." -ForegroundColor Yellow
  & rustup update stable
  & rustup default stable
  $rust = Version-De "rustc" "--version"
}
if (-not $rust -or $rust -lt [version]"1.88") { Erreur "Rust 1.88 ou plus récent est nécessaire (trouvé : $rust)."; exit 1 }
Ok "Rust $rust"

# 3. Node.js 20 ou plus récent.
$node = if (Present "node") { Version-De "node" "--version" } else { $null }
if (-not $node -or $node -lt [version]"20.0") {
  Installer "Node.js (LTS)" "OpenJS.NodeJS.LTS"
  $node = Version-De "node" "--version"
}
if (-not $node -or -not (Present "npm")) { Erreur "Node.js reste introuvable après l'installation : rouvrez la console et relancez."; exit 1 }
Ok "Node.js $node"

# 4. WebView2 : présent sur tout Windows 10 ou 11 à jour ; sans lui, l'application ne pourrait pas s'ouvrir.
if (WebView2-Present) { Ok "WebView2 : présent" } else { Installer "WebView2" "Microsoft.EdgeWebView2Runtime"; Ok "WebView2 : installé" }

# ---------------------------------------------------------------- version
if ($Version) {
  if ($Version -notmatch '^\d+\.\d+\.\d+$') { Erreur "Version attendue sous la forme 0.2.0 (reçu : $Version)."; exit 1 }
  Titre "Version $Version"
  foreach ($fichier in "src-tauri\tauri.conf.json", "package.json") {
    $texte = Get-Content $fichier -Raw -Encoding UTF8
    # Seule la première ligne "version" (celle de l'application) est touchée.
    $regex = [regex]'"version"\s*:\s*"[^"]*"'
    $texte = $regex.Replace($texte, "`"version`": `"$Version`"", 1)
    [System.IO.File]::WriteAllText((Join-Path $racine $fichier), $texte, (New-Object System.Text.UTF8Encoding($false)))
  }
  Ok "tauri.conf.json et package.json à jour"
}

# ---------------------------------------------------------------- construction
Titre "Dépendances de l'interface"
Write-Host "  Dossier du projet : $racine"
if (-not (Test-Path (Join-Path $racine "package.json"))) {
  Erreur "package.json est introuvable : ce script doit rester dans le dossier export\ du projet."
  exit 1
}
# L'outil de construction est un programme natif chargé par Node. Le seul test qui vaille est de le
# CHARGER : un dossier présent ne prouve rien, car une installation interrompue (ou deux installations
# lancées en même temps) laisse un fichier à moitié écrit — Windows répond alors « n'est pas une
# application Win32 valide ». En cas de doute, tout est refait : c'est l'affaire d'une minute.
$outil = Join-Path $racine "node_modules\@tauri-apps\cli\tauri.js"
function Outil-Fonctionne {
  if (-not (Test-Path $outil)) { return $false }
  # Demander sa version charge le programme natif : s'il est absent ou abîmé, la commande échoue.
  & node $outil --version 2>$null | Out-Null
  return $LASTEXITCODE -eq 0
}

function Installer-Dependances {
  # package-lock.json est conservé : c'est lui qui fixe les versions des paquets Tauri côté npm,
  # alignées sur celles des crates Rust (Cargo.lock). Réinstaller « au plus récent » les ferait
  # diverger (2.11 d'un côté, 2.12 de l'autre) et `tauri build` refuse alors de démarrer.
  foreach ($reste in "node_modules") {
    if (-not (Test-Path $reste)) { continue }
    # Remove-Item bute parfois sur les chemins longs ou un fichier tenu par l'antivirus : rmdir prend le relais.
    try { Remove-Item $reste -Recurse -Force -ErrorAction Stop } catch { cmd /c rmdir /s /q "$reste" 2>$null; cmd /c del /f /q "$reste" 2>$null }
    if (Test-Path $reste) {
      Erreur "Impossible de supprimer $reste : fermez ce qui l'utilise (éditeur, explorateur, autre console) et relancez."
      exit 1
    }
  }
  # `npm ci` installe exactement ce que décrit package-lock.json (repli sur `npm install` s'il manque).
  # Deux essais : un antivirus qui inspecte les fichiers fait parfois échouer le premier.
  $commande = if (Test-Path "package-lock.json") { "ci" } else { "install" }
  npm $commande
  if ($LASTEXITCODE -ne 0) {
    Write-Host "  Première tentative échouée ; nouvel essai..." -ForegroundColor Yellow
    Start-Sleep -Seconds 3
    npm $commande
  }
  if ($LASTEXITCODE -ne 0) { Erreur "npm $commande a échoué (antivirus, OneDrive ou coupure réseau ?)."; exit $LASTEXITCODE }
}

if ($Reinstaller) {
  Write-Host "  Réinstallation demandée." -ForegroundColor Yellow
  Installer-Dependances
} elseif (-not (Test-Path "node_modules")) {
  Write-Host "  Première installation : quelques minutes." -ForegroundColor Yellow
  Installer-Dependances
} elseif (-not (Outil-Fonctionne)) {
  Write-Host "  Dépendances incomplètes ou abîmées : elles sont refaites." -ForegroundColor Yellow
  Installer-Dependances
}
if (-not (Outil-Fonctionne)) {
  Erreur "L'outil de construction ne se charge toujours pas après réinstallation."
  Write-Host "  Essayez, depuis le dossier du projet :  npm cache clean --force  puis relancez ce script."
  exit 1
}
Ok "Dépendances prêtes"

# Polices incluses (src-tauri\fonts) : sans elles, Tauri refuse de compiler (ressource `fonts/**/*` introuvable).
if (-not (Get-ChildItem -Path (Join-Path $racine "src-tauri\fonts") -Recurse -Include *.ttf,*.otf -ErrorAction SilentlyContinue | Select-Object -First 1)) {
  Erreur "Le dossier src-tauri\fonts est vide ou absent : les polices incluses manquent."
  Write-Host "  Décompressez l'archive complète du projet (les polices en font partie), puis relancez."
  exit 1
}

Titre "Modèles (dossier modeles\)"
& (Join-Path $PSScriptRoot "sync-modeles.ps1")

if ($Essai) {
  Titre "Compilation pour essai - sans installateur"
  & node $outil build --no-bundle
  if ($LASTEXITCODE -ne 0) { Erreur "La compilation a échoué : copiez les dernières lignes ci-dessus pour les signaler."; exit $LASTEXITCODE }
  $essai = Join-Path $racine "target\release\lumitype.exe"
  if (-not (Test-Path $essai)) { $essai = Join-Path $racine "target\release\image-editor.exe" }
  Ok "Lancement de $essai"
  Start-Process -FilePath $essai -WorkingDirectory (Split-Path $essai)
  exit 0
}

Titre "Construction (installateurs NSIS et MSI) - dix à vingt minutes la première fois"
$debut = Get-Date
# L'outil est appelé par son script, sans passer par un raccourci ni par le PATH : c'est ce qui manquait
# quand la console répondait « 'tauri' n'est pas reconnu ».
& node $outil build --bundles nsis,msi
if ($LASTEXITCODE -ne 0) { Erreur "La construction a échoué : copiez les dernières lignes ci-dessus pour les signaler."; exit $LASTEXITCODE }

# ---------------------------------------------------------------- rangement
Titre "Rangement dans export\sorties\"
New-Item -ItemType Directory -Force -Path $sorties | Out-Null
$trouves = 0
foreach ($cible in "target\release", "src-tauri\target\release") {
  $dossier = Join-Path $racine $cible
  if (-not (Test-Path $dossier)) { continue }
  $paquets = Get-ChildItem -Path (Join-Path $dossier "bundle") -Recurse -Include "*-setup.exe", "*.msi" -ErrorAction SilentlyContinue |
    Where-Object { $_.LastWriteTime -ge $debut }
  foreach ($fichier in $paquets) {
    Copy-Item $fichier.FullName $sorties -Force
    Write-Host ("  {0}  ({1:N1} Mo)" -f $fichier.Name, ($fichier.Length / 1MB))
    $trouves++
  }
  # L'exécutable seul : fonctionne sans installation sur tout Windows 10 ou 11 à jour (WebView2 y est déjà présent).
  $binaire = Get-ChildItem -Path $dossier -Filter "lumitype.exe" -File -ErrorAction SilentlyContinue |
    Where-Object { $_.LastWriteTime -ge $debut } | Select-Object -First 1
  if ($binaire) {
    Copy-Item $binaire.FullName (Join-Path $sorties "Lumitype-portable.exe") -Force
    Write-Host ("  Lumitype-portable.exe  ({0:N1} Mo)" -f ($binaire.Length / 1MB))
    $trouves++
  }
}
if ($trouves -eq 0) { Erreur "Aucun fichier trouvé : la construction a dû échouer plus haut."; exit 1 }

Write-Host "`nTerminé. À distribuer : le fichier ...-setup.exe de export\sorties\" -ForegroundColor Green
Write-Host "Pour mettre à jour Lumitype sur ce PC : fermez-le, puis lancez ce -setup.exe."
Write-Host "Sans signature de code, Windows affichera 'éditeur inconnu' : Informations complémentaires > Exécuter quand même."
