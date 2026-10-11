// Polices incluses avec Lumitype (src-tauri/fonts) : le moteur les connaît par leur fichier ; la webview, elle, ne
// voit que les polices du système. Pour afficher un échantillon ou un aperçu dans ces polices, leur fichier est
// chargé une fois dans la page (`FontFace`), sous le même nom de famille que celui du moteur.

const loaded = new Map<string, Promise<void>>();

/** Rend une police incluse utilisable en CSS (`font-family: "<famille>"`). Sans effet si elle l'est déjà. */
export function ensureBundledFont(family: string, fetchFile: () => Promise<ArrayBuffer>): Promise<void> {
  const pending = loaded.get(family);
  if (pending) return pending;
  const task = fetchFile()
    .then(async (bytes) => {
      if (bytes.byteLength === 0) return; // police du système, ou inconnue : rien à charger
      const face = new FontFace(family, bytes);
      await face.load();
      document.fonts.add(face);
    })
    .catch((error: unknown) => {
      loaded.delete(family); // nouvel essai possible plus tard
      console.warn("Police incluse non chargée", family, error);
    });
  loaded.set(family, task);
  return task;
}
