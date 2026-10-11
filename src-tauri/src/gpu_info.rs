//! Mémoire vidéo de la carte graphique, pour le réglage « Mémoire vidéo utilisable » des préférences.
//!
//! La webview ne dit rien de la mémoire de la carte (WebGPU ne l'expose pas) : on la lit dans le système, dans
//! l'ordre, en s'arrêtant au premier qui répond :
//! 1. **Vulkan** (crate `ash`) : le plus grand tas de mémoire « locale à la carte » de la carte la mieux dotée.
//!    Le chargeur Vulkan est ouvert à l'exécution (`Entry::load`), sans lien statique : sans Vulkan sur la machine,
//!    l'échec est silencieux. Sur une carte intégrée, ce tas est pris sur la mémoire vive : il est signalé
//!    comme *partagé* ;
//! 2. **Linux** : `mem_info_vram_total` dans sysfs (pilote amdgpu ; la plus grosse carte l'emporte) ;
//! 3. **Windows** : DXGI, la mémoire dédiée (`DedicatedVideoMemory`) de la carte la mieux dotée, cartes logicielles
//!    (WARP) exclues ;
//! 4. sinon `None` : taille inconnue, l'interface suppose alors 2 Gio et le dit.
//!
//! Une carte intégrée (portable) se reconnaît à une mémoire dédiée petite (< 1 Gio), partagée ou inconnue : sa mémoire
//! vidéo est prise sur la mémoire vive, et l'interface borne alors le plafond à un quart de celle-ci.
//!
//! La détection est faite une fois par processus (`OnceLock`) : ouvrir Vulkan coûte quelques dizaines de millisecondes.

use std::sync::OnceLock;

/// Mémoire vidéo d'une carte graphique, en mébioctets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VideoMemory {
    pub mb: u64,
    /// Carte intégrée : sa mémoire « vidéo » est un morceau de la mémoire vive.
    pub shared: bool,
}

/// Mémoire vidéo de la carte graphique, si le système la donne.
pub fn video_memory() -> Option<VideoMemory> {
    static DETECTED: OnceLock<Option<VideoMemory>> = OnceLock::new();
    *DETECTED.get_or_init(detect)
}

fn detect() -> Option<VideoMemory> {
    vulkan_video_memory().or_else(system_video_memory)
}

/// Lecture propre au système, sans Vulkan.
fn system_video_memory() -> Option<VideoMemory> {
    #[cfg(target_os = "linux")]
    {
        linux_sysfs_vram_mb().map(|mb| VideoMemory { mb, shared: false })
    }
    #[cfg(windows)]
    {
        dxgi_vram_mb().map(|mb| VideoMemory { mb, shared: false })
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        None
    }
}

/// Vulkan : parmi les cartes physiques (cartes logicielles exclues), la carte dédiée au plus grand tas local, sinon
/// la carte intégrée au plus grand tas (signalée comme partagée). Tout échec (pas de chargeur, pas de pilote, pas de
/// carte) donne `None`, sans message : c'est un cas normal.
fn vulkan_video_memory() -> Option<VideoMemory> {
    use ash::vk;
    // SÉCURITÉ : `Entry::load` ouvre la bibliothèque Vulkan du système ; les fonctions appelées ensuite sont
    // celles du chargeur, avec des structures construites ici et détruites avant la sortie.
    let entry = unsafe { ash::Entry::load() }.ok()?;
    let app_info = vk::ApplicationInfo::default().application_name(c"Lumitype").api_version(vk::API_VERSION_1_0);
    let create_info = vk::InstanceCreateInfo::default().application_info(&app_info);
    let instance = unsafe { entry.create_instance(&create_info, None) }.ok()?;
    let devices = unsafe { instance.enumerate_physical_devices() }.unwrap_or_default();
    let (mut discrete, mut integrated) = (None, None);
    for device in devices {
        let properties = unsafe { instance.get_physical_device_properties(device) };
        let memory = unsafe { instance.get_physical_device_memory_properties(device) };
        let heaps = &memory.memory_heaps[..(memory.memory_heap_count as usize).min(memory.memory_heaps.len())];
        let largest = heaps.iter().filter(|heap| heap.flags.contains(vk::MemoryHeapFlags::DEVICE_LOCAL)).map(|heap| heap.size).max().unwrap_or(0);
        let mb = largest / (1024 * 1024);
        let slot = match properties.device_type {
            vk::PhysicalDeviceType::DISCRETE_GPU | vk::PhysicalDeviceType::VIRTUAL_GPU => &mut discrete,
            vk::PhysicalDeviceType::INTEGRATED_GPU => &mut integrated,
            // CPU (llvmpipe, SwiftShader) ou inconnu : pas une carte.
            _ => continue,
        };
        *slot = Some(slot.map_or(mb, |known: u64| known.max(mb)));
    }
    unsafe { instance.destroy_instance(None) };
    match (discrete, integrated) {
        (Some(mb), _) if mb > 0 => Some(VideoMemory { mb, shared: false }),
        (_, Some(mb)) if mb > 0 => Some(VideoMemory { mb, shared: true }),
        _ => None,
    }
}

/// amdgpu (et quelques autres pilotes) publient la mémoire de la carte dans sysfs, en octets.
#[cfg(target_os = "linux")]
fn linux_sysfs_vram_mb() -> Option<u64> {
    let entries = std::fs::read_dir("/sys/class/drm").ok()?;
    entries
        .flatten()
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            // `card0`, `card1`… mais pas leurs sorties d'écran (`card0-DP-1`).
            name.starts_with("card") && !name.contains('-')
        })
        .filter_map(|entry| std::fs::read_to_string(entry.path().join("device/mem_info_vram_total")).ok())
        .filter_map(|text| text.trim().parse::<u64>().ok())
        .map(|bytes| bytes / (1024 * 1024))
        .filter(|mb| *mb > 0)
        .max()
}

/// DXGI énumère les cartes avec leur mémoire dédiée ; on retient la plus dotée, cartes logicielles (WARP) exclues.
/// `DXGI_ADAPTER_DESC1.DedicatedVideoMemory` suffit (la mémoire dédiée est fixe) ; `QueryVideoMemoryInfo` ne
/// donnerait en plus que l'occupation du moment, dont on n'a pas besoin.
#[cfg(windows)]
fn dxgi_vram_mb() -> Option<u64> {
    use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE};
    // SÉCURITÉ : appels COM documentés, sans pointeur fourni par nous ; les objets sont libérés à la fin de la portée.
    let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1() }.ok()?;
    let mut best = None;
    for index in 0.. {
        let Ok(adapter) = (unsafe { factory.EnumAdapters1(index) }) else { break };
        let Ok(desc) = (unsafe { adapter.GetDesc1() }) else { continue };
        if desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 {
            continue;
        }
        let mb = desc.DedicatedVideoMemory as u64 / (1024 * 1024);
        if mb == 0 {
            continue;
        }
        best = Some(best.map_or(mb, |known: u64| known.max(mb)));
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detection_never_panics_and_is_stable() {
        // Sur la machine de test il peut n'y avoir ni Vulkan ni carte : la réponse doit juste être cohérente et la même à chaque appel.
        let first = video_memory();
        assert_eq!(first, video_memory());
        if let Some(memory) = first {
            assert!(memory.mb > 0);
        }
    }
}
