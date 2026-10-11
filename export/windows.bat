@echo off
rem Construit Lumitype pour Windows (double-clic). Installe au besoin ce qui manque (Build Tools, Rust,
rem Node.js) et refait les dépendances de l'interface si elles sont incomplètes.
rem Les fichiers arrivent dans export\sorties\
rem
rem   windows.bat                 construit
rem   windows.bat 0.3.7           fixe d'abord la version, puis construit
rem   windows.bat -Reinstaller    refait les dépendances de zéro, puis construit
rem   windows.bat -Essai          compile sans installateur, puis lance Lumitype pour essayer
rem
rem Le dossier de travail n'a aucune importance : le script se place tout seul dans celui du projet.
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0windows.ps1" %*
echo.
pause
