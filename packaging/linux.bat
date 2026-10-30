@echo off
rem Lance packaging/linux.sh depuis Windows, dans WSL 2 : les paquets Linux sortent dans packaging\out\, comme les
rem fichiers Windows. Les options sont celles de linux.sh :
rem
rem   linux.bat                        tous les formats possibles dans WSL
rem   linux.bat deb appimage           seulement ceux-la (deb rpm arch flatpak appimage tar)
rem   linux.bat 1.3.0 --release        fixe la version, construit, range sous les noms de diffusion + SHA256SUMS.txt
rem   linux.bat --oui                  installe sans demander ce qui manque (apt, dans WSL)
rem
rem Compatibilite : un paquet Linux ne demarre que sur une glibc au moins aussi recente que celle de la distribution
rem qui l'a construit. Ubuntu-22.04 (glibc 2.35) est donc choisie d'office si elle est installee : ses paquets
rem demarrent sur Linux Mint 21+, Ubuntu 22.04+, Debian 12+, Fedora, Arch. L'Ubuntu par defaut de WSL (24.04, glibc
rem 2.39) exclut Mint 21 et Debian 12. Pour l'ajouter, en administrateur :  wsl --install -d Ubuntu-22.04
rem Autre distribution WSL :  set LUMITYPE_WSL=Ubuntu-24.04  puis  linux.bat
rem
rem Premiere fois : WSL demande le mot de passe sudo de la distribution pour installer les outils de construction.
rem La compilation dans un dossier Windows (/mnt/c/...) est plus lente qu'un dossier Linux : pour aller plus vite,
rem clonez le projet dans WSL (ex. ~/lumitype) et lancez ./packaging/linux.sh depuis la.
rem
rem WSL absent : ouvrez PowerShell en administrateur, tapez  wsl --install -d Ubuntu-22.04  , redemarrez, puis relancez.
setlocal
where wsl >nul 2>&1
if errorlevel 1 (
  echo WSL n'est pas installe. En administrateur :  wsl --install -d Ubuntu-22.04   puis redemarrez Windows.
  pause
  exit /b 1
)
set "AUCUNE=1"
for /f "usebackq delims=" %%d in (`wsl -l -q 2^>nul ^| findstr /r /v "^$"`) do set "AUCUNE=0"
if "%AUCUNE%"=="1" (
  echo Aucune distribution Linux dans WSL. En administrateur :  wsl --install -d Ubuntu-22.04
  pause
  exit /b 1
)
rem Distribution utilisee : LUMITYPE_WSL si fixee, sinon Ubuntu-22.04 si presente, sinon celle par defaut de WSL.
set "CHOIX=%LUMITYPE_WSL%"
if not defined CHOIX (
  wsl -d Ubuntu-22.04 -e true >nul 2>&1
  if not errorlevel 1 set "CHOIX=Ubuntu-22.04"
)
if defined CHOIX (
  echo Construction des paquets Linux dans WSL : %CHOIX%
  set "WSL_DISTRO=-d %CHOIX%"
) else (
  echo Construction des paquets Linux dans la distribution WSL par defaut.
  echo   Attention : si c'est une Ubuntu 24.04 ou plus recente, les paquets ne demarreront pas sur Linux Mint 21 ni Debian 12.
  echo   Pour des paquets compatibles partout :  wsl --install -d Ubuntu-22.04   puis relancez ce script.
  set "WSL_DISTRO="
)
rem Le dossier du projet (parent de packaging\) est traduit en chemin Linux par wslpath ; les options sont transmises.
rem linux.sh affiche ensuite le nom exact de la distribution, sa glibc, et les distributions ou les paquets demarrent.
wsl %WSL_DISTRO% -e bash -lc "cd \"$(wslpath -u '%~dp0..')\" && bash ./packaging/linux.sh %*"
set "CODE=%errorlevel%"
echo.
if "%CODE%"=="0" (echo Termine : paquets Linux dans packaging\out\) else (echo Echec, code %CODE% : voir les messages ci-dessus.)
pause
exit /b %CODE%
