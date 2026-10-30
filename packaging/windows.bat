@echo off
rem Construit Lumitype pour Windows (double-clic). Installe au besoin ce qui manque (Build Tools, Rust,
rem Node.js, NASM) et refait les dépendances de l'interface si elles sont incomplètes.
rem Les fichiers arrivent dans packaging\out\
rem
rem   windows.bat                 construit
rem   windows.bat 0.3.7           fixe d'abord la version, puis construit
rem   windows.bat -Reinstaller    refait les dépendances de zéro, puis construit
rem   windows.bat -Essai          compile sans installateur, puis lance Lumitype pour essayer
rem   windows.bat -Release        copie aussi les 3 fichiers sous leur nom de diffusion + SHA256SUMS.txt
rem
rem Les paquets Linux (deb, rpm, AppImage, Flatpak...) se font avec linux.bat (WSL 2) ou packaging/linux.sh sous Linux.
rem   windows.bat -ExclureAntivirus  (en administrateur) retire le dossier de l'analyse de Defender
rem                               sans poser la question ; cause habituelle de « Accès refusé (os error 5) »
rem
rem Le dossier de travail n'a aucune importance : le script se place tout seul dans celui du projet.
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0windows.ps1" %*
echo.
pause
