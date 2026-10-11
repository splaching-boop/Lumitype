import re,glob,json,sys
root='ui/src/'
keys=[]
def add(k):
    k=k.strip()
    if k and k not in keys: keys.append(k)
# 1. appels t("…") / t('…')
for p in glob.glob(root+'**/*.svelte',recursive=True)+glob.glob(root+'**/*.ts',recursive=True):
    if 'locales/' in p: continue
    s=open(p).read()
    for m in re.finditer(r'\bt\(\s*"((?:[^"\\]|\\.)*)"',s): add(m.group(1).replace('\\"','"'))
    for m in re.finditer(r"\bt\(\s*'((?:[^'\\]|\\.)*)'",s): add(m.group(1).replace("\\'","'"))
# 2. catalogues : toutes les valeurs string françaises de certains fichiers
CAT=['lib/action-catalog.ts','lib/filters.ts','lib/tool-help.ts','lib/blend.ts','lib/gradient.ts','lib/news.ts','lib/guide-templates.ts','lib/keymap.ts','lib/articles/adjustments.ts','lib/articles/masks.ts','lib/articles/tools.ts','components/Toolbar.svelte','components/LayersPanel.svelte','components/HomeScreen.svelte','components/EffectsDialog.svelte','components/NewDocumentDialog.svelte','components/ExportDialog.svelte','components/AdjustmentProperties.svelte','components/LiquifyDialog.svelte','components/BrushPicker.svelte','components/BrushQuickMenu.svelte','components/ColorPanel.svelte','components/ShapeProperties.svelte','components/GuideLayoutDialog.svelte','components/OptionsBar.svelte','components/StatusBar.svelte','components/PreferencesDialog.svelte','components/TextProperties.svelte','App.svelte','lib/gpu-plan.ts','lib/actions.ts','viewport/PathOverlay.svelte','components/MenuBar.svelte','components/FilterDialog.svelte','components/HomeButton.svelte','components/DocTabs.svelte','components/Panel.svelte','components/RecoveryDialog.svelte','components/SavePrompt.svelte','components/ShortcutsDialog.svelte','components/ModifySelectionDialog.svelte','components/FeatherDialog.svelte','components/ShapeDialog.svelte','components/FontPicker.svelte','components/ContextMenu.svelte','components/SplashScreen.svelte','viewport/Viewport.svelte','viewport/TransformOverlay.svelte','lib/autosave.svelte.ts','lib/recents.svelte.ts','state/app.svelte.ts','lib/transform-session.ts','lib/text-edit.svelte.ts','lib/engine.ts']
LET=re.compile(r"[a-zà-ÿœ]")
for rel in CAT:
    p=root+rel; s=open(p).read()
    if p.endswith('.svelte'):
        m=re.search(r"<script[^>]*>(.*?)</script>",s,re.S); s=m.group(1) if m else ""
    s=re.sub(r"^\s*//.*$","",s,flags=re.M); s=re.sub(r"/\*.*?\*/","",s,flags=re.S)
    for m in re.finditer(r'(?:(\w+)\s*:\s*|=\s*|\(|,\s*|\[\s*|\?\s*|:\s*)"((?:[^"\\]|\\.)*)"',s):
        field,lit=m.group(1),m.group(2).replace('\\"','"')
        if field in ('id','key','op','type','kind','icon','action','extension','value','target','mode','shortcut','dynamic','image','gif','demo','figure','name','chord','tag','code','extensions','unit','blend','version','date','hero','illustration','level'): 
            if field!='name' or not (' ' in lit or re.search('[éèàç]',lit)): continue
        if not LET.search(lit) or re.match(r'^M[\d.-]',lit) or lit.startswith(('../','./','http','#','image/','data:')) or re.match(r'^[a-z][\w.-]*$',lit): continue
        if not (' ' in lit or re.search('[éèàçêù]',lit) or lit[0].isupper()): continue
        add(lit)
# 3. libellés venant du moteur (historique, noms de calque)
for p in ['src-tauri/src/commands.rs','crates/engine/src/document.rs','crates/engine/src/adjust.rs','crates/engine/src/filters.rs','crates/engine/src/effects.rs','crates/engine/src/session.rs','crates/engine/src/io.rs']:
    s=open(p).read()
    for m in re.finditer(r'session\.apply\(\s*"((?:[^"\\]|\\.)*)"',s): add(m.group(1))
    for m in re.finditer(r'(?:apply|record|label|name)\w*\(\s*(?:&)?(?:format!\()?"((?:[^"\\]|\\.)*)"',s): 
        lit=m.group(1)
        if ' ' in lit or re.search('[éèàç]',lit): add(lit)
    for m in re.finditer(r'=> "((?:[^"\\]|\\.)*)"',s): add(m.group(1))
    for m in re.finditer(r'InvalidOperation\((?:format!\()?"((?:[^"\\]|\\.)*)"',s): add(m.group(1))
add("Arrière-plan"); add("Sans titre"); add("Calque"); add("Groupe")
json.dump(keys,open(sys.argv[1] if len(sys.argv)>1 else "ui/src/lib/locales/keys.fr.json","w"),ensure_ascii=False,indent=0)
print(len(keys),'clés')
