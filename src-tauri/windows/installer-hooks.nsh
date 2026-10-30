;; Ajouts de Lumitype à l'installateur NSIS (bundle > windows > nsis > installerHooks, inclus par installer.nsi).
;;
;; Visionneuse Lumitype : `lumitype-visionneuse.exe`, lien dur vers lumitype.exe (le même programme, qui se lance en
;; visionneuse d'après son nom : voir src-tauri/src/main.rs, `viewer_mode`) ; copie si le lien est impossible (disque
;; qui ne les gère pas). Raccourci « Lumitype Visionneuse » dans le menu Démarrer, entrée « Ouvrir avec » pour les
;; images, et application déclarée dans Paramètres > Applications > Applications par défaut. Rien n'est imposé :
;; l'utilisateur choisit lui-même la visionneuse par défaut.

!define LUMITYPE_VIEWER_EXE "lumitype-visionneuse.exe"
!define LUMITYPE_VIEWER_NAME "Lumitype Visionneuse"
!define LUMITYPE_VIEWER_PROGID "Lumitype.Visionneuse"
!define LUMITYPE_VIEWER_CAPS "Software\Lumitype\Visionneuse\Capabilities"
;; Icônes de l'Explorateur. Projets (.graf) : la page « pellicule » de Lumitype (la même que sous Linux,
;; linux/mimetypes), et non le logo de l'application. Images ouvertes par la visionneuse : l'icône d'image de Windows
;; (celle des JPEG), comme pour les autres visionneuses — l'Explorateur montre de toute façon l'aperçu quand il sait le
;; calculer. Dossier de ce fichier, lu ici (dans une macro, __FILEDIR__ serait celui de installer.nsi).
!define LUMITYPE_HOOKS_DIR "${__FILEDIR__}"
!define LUMITYPE_PROJECT_ICON "projet-lumitype.ico"
!define LUMITYPE_PROJECT_CLASS "Projet Lumitype"
!define LUMITYPE_IMAGE_ICON "%SystemRoot%\System32\imageres.dll,-72"

!macro LUMITYPE_VIEWER_EXT ext
  WriteRegStr SHCTX "Software\Classes\Applications\${LUMITYPE_VIEWER_EXE}\SupportedTypes" "${ext}" ""
  WriteRegStr SHCTX "Software\Classes\${ext}\OpenWithProgids" "${LUMITYPE_VIEWER_PROGID}" ""
  WriteRegStr SHCTX "${LUMITYPE_VIEWER_CAPS}\FileAssociations" "${ext}" "${LUMITYPE_VIEWER_PROGID}"
!macroend

!macro LUMITYPE_VIEWER_UNEXT ext
  DeleteRegValue SHCTX "Software\Classes\${ext}\OpenWithProgids" "${LUMITYPE_VIEWER_PROGID}"
!macroend

; Avant la copie des fichiers : une visionneuse ouverte tient le même fichier que lumitype.exe (lien dur) ; elle est
; fermée (avec l'accord de l'utilisateur), puis le lien est retiré pour que lumitype.exe puisse être remplacé.
!macro NSIS_HOOK_PREINSTALL
  !insertmacro CheckIfAppIsRunning "$INSTDIR\${LUMITYPE_VIEWER_EXE}" "${LUMITYPE_VIEWER_NAME}"
  Delete "$INSTDIR\${LUMITYPE_VIEWER_EXE}"
!macroend

!macro NSIS_HOOK_POSTINSTALL
  Delete "$INSTDIR\${LUMITYPE_VIEWER_EXE}"
  System::Call 'kernel32::CreateHardLinkW(w "$INSTDIR\${LUMITYPE_VIEWER_EXE}", w "$INSTDIR\${MAINBINARYNAME}.exe", p 0) i .r0'
  ${If} $0 = 0
    CopyFiles /SILENT "$INSTDIR\${MAINBINARYNAME}.exe" "$INSTDIR\${LUMITYPE_VIEWER_EXE}"
  ${EndIf}
  CreateShortcut "$SMPROGRAMS\${LUMITYPE_VIEWER_NAME}.lnk" "$INSTDIR\${LUMITYPE_VIEWER_EXE}" "" "$INSTDIR\${LUMITYPE_VIEWER_EXE}" 0
  ; « Ouvrir avec » : le nom affiché vient de FriendlyAppName.
  WriteRegStr SHCTX "Software\Classes\Applications\${LUMITYPE_VIEWER_EXE}" "FriendlyAppName" "${LUMITYPE_VIEWER_NAME}"
  WriteRegExpandStr SHCTX "Software\Classes\Applications\${LUMITYPE_VIEWER_EXE}\DefaultIcon" "" "${LUMITYPE_IMAGE_ICON}"
  WriteRegStr SHCTX "Software\Classes\Applications\${LUMITYPE_VIEWER_EXE}\shell\open\command" "" "$\"$INSTDIR\${LUMITYPE_VIEWER_EXE}$\" $\"%1$\""
  ; Type de fichier de la visionneuse (applications par défaut).
  WriteRegStr SHCTX "Software\Classes\${LUMITYPE_VIEWER_PROGID}" "" "Image"
  WriteRegExpandStr SHCTX "Software\Classes\${LUMITYPE_VIEWER_PROGID}\DefaultIcon" "" "${LUMITYPE_IMAGE_ICON}"
  WriteRegStr SHCTX "Software\Classes\${LUMITYPE_VIEWER_PROGID}\Application" "ApplicationName" "${LUMITYPE_VIEWER_NAME}"
  WriteRegStr SHCTX "Software\Classes\${LUMITYPE_VIEWER_PROGID}\shell\open\command" "" "$\"$INSTDIR\${LUMITYPE_VIEWER_EXE}$\" $\"%1$\""
  WriteRegStr SHCTX "${LUMITYPE_VIEWER_CAPS}" "ApplicationName" "${LUMITYPE_VIEWER_NAME}"
  WriteRegStr SHCTX "${LUMITYPE_VIEWER_CAPS}" "ApplicationDescription" "Visionneuse d'images de Lumitype : JPEG, PNG, WebP, AVIF, TIFF, HEIC et RAW."
  WriteRegStr SHCTX "${LUMITYPE_VIEWER_CAPS}" "ApplicationIcon" "$INSTDIR\${LUMITYPE_VIEWER_EXE},0"
  !insertmacro LUMITYPE_VIEWER_EXT ".jpg"
  !insertmacro LUMITYPE_VIEWER_EXT ".jpeg"
  !insertmacro LUMITYPE_VIEWER_EXT ".jfif"
  !insertmacro LUMITYPE_VIEWER_EXT ".png"
  !insertmacro LUMITYPE_VIEWER_EXT ".webp"
  !insertmacro LUMITYPE_VIEWER_EXT ".avif"
  !insertmacro LUMITYPE_VIEWER_EXT ".gif"
  !insertmacro LUMITYPE_VIEWER_EXT ".bmp"
  !insertmacro LUMITYPE_VIEWER_EXT ".tif"
  !insertmacro LUMITYPE_VIEWER_EXT ".tiff"
  !insertmacro LUMITYPE_VIEWER_EXT ".heic"
  !insertmacro LUMITYPE_VIEWER_EXT ".heif"
  !insertmacro LUMITYPE_VIEWER_EXT ".dng"
  !insertmacro LUMITYPE_VIEWER_EXT ".arw"
  !insertmacro LUMITYPE_VIEWER_EXT ".cr2"
  !insertmacro LUMITYPE_VIEWER_EXT ".cr3"
  !insertmacro LUMITYPE_VIEWER_EXT ".nef"
  !insertmacro LUMITYPE_VIEWER_EXT ".nrw"
  !insertmacro LUMITYPE_VIEWER_EXT ".raf"
  !insertmacro LUMITYPE_VIEWER_EXT ".orf"
  !insertmacro LUMITYPE_VIEWER_EXT ".rw2"
  !insertmacro LUMITYPE_VIEWER_EXT ".pef"
  !insertmacro LUMITYPE_VIEWER_EXT ".srw"
  WriteRegStr SHCTX "Software\RegisteredApplications" "${LUMITYPE_VIEWER_NAME}" "${LUMITYPE_VIEWER_CAPS}"
  ; Projets : icône de document (l'association .graf, écrite juste avant par Tauri, pointait sur lumitype.exe).
  File "/oname=$INSTDIR\${LUMITYPE_PROJECT_ICON}" "${LUMITYPE_HOOKS_DIR}\${LUMITYPE_PROJECT_ICON}"
  WriteRegStr SHCTX "Software\Classes\${LUMITYPE_PROJECT_CLASS}\DefaultIcon" "" "$INSTDIR\${LUMITYPE_PROJECT_ICON}"
  ; Prévient l'Explorateur que les associations ont changé.
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)'
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  !insertmacro CheckIfAppIsRunning "$INSTDIR\${LUMITYPE_VIEWER_EXE}" "${LUMITYPE_VIEWER_NAME}"
  Delete "$INSTDIR\${LUMITYPE_VIEWER_EXE}"
  Delete "$SMPROGRAMS\${LUMITYPE_VIEWER_NAME}.lnk"
  Delete "$INSTDIR\${LUMITYPE_PROJECT_ICON}"
  !insertmacro LUMITYPE_VIEWER_UNEXT ".jpg"
  !insertmacro LUMITYPE_VIEWER_UNEXT ".jpeg"
  !insertmacro LUMITYPE_VIEWER_UNEXT ".jfif"
  !insertmacro LUMITYPE_VIEWER_UNEXT ".png"
  !insertmacro LUMITYPE_VIEWER_UNEXT ".webp"
  !insertmacro LUMITYPE_VIEWER_UNEXT ".avif"
  !insertmacro LUMITYPE_VIEWER_UNEXT ".gif"
  !insertmacro LUMITYPE_VIEWER_UNEXT ".bmp"
  !insertmacro LUMITYPE_VIEWER_UNEXT ".tif"
  !insertmacro LUMITYPE_VIEWER_UNEXT ".tiff"
  !insertmacro LUMITYPE_VIEWER_UNEXT ".heic"
  !insertmacro LUMITYPE_VIEWER_UNEXT ".heif"
  !insertmacro LUMITYPE_VIEWER_UNEXT ".dng"
  !insertmacro LUMITYPE_VIEWER_UNEXT ".arw"
  !insertmacro LUMITYPE_VIEWER_UNEXT ".cr2"
  !insertmacro LUMITYPE_VIEWER_UNEXT ".cr3"
  !insertmacro LUMITYPE_VIEWER_UNEXT ".nef"
  !insertmacro LUMITYPE_VIEWER_UNEXT ".nrw"
  !insertmacro LUMITYPE_VIEWER_UNEXT ".raf"
  !insertmacro LUMITYPE_VIEWER_UNEXT ".orf"
  !insertmacro LUMITYPE_VIEWER_UNEXT ".rw2"
  !insertmacro LUMITYPE_VIEWER_UNEXT ".pef"
  !insertmacro LUMITYPE_VIEWER_UNEXT ".srw"
  DeleteRegKey SHCTX "Software\Classes\Applications\${LUMITYPE_VIEWER_EXE}"
  DeleteRegKey SHCTX "Software\Classes\${LUMITYPE_VIEWER_PROGID}"
  DeleteRegValue SHCTX "Software\RegisteredApplications" "${LUMITYPE_VIEWER_NAME}"
  DeleteRegKey SHCTX "Software\Lumitype\Visionneuse"
  DeleteRegKey /ifempty SHCTX "Software\Lumitype"
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)'
!macroend
