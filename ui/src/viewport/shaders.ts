// Sommets communs à tous les programmes de l'affichage : un rectangle `u_rect` (pixels physiques, origine en haut à
// gauche) de l'écran de taille `u_view`, avec les coordonnées de texture `u_uv`.
export const VERTEX_SHADER = `#version 300 es
layout(location = 0) in vec2 a_pos;
uniform vec4 u_rect;   // x, y, largeur, hauteur en pixels physiques
uniform vec4 u_uv;     // u0, v0, du, dv
uniform vec2 u_view;
out vec2 v_uv;
void main() {
  v_uv = u_uv.xy + a_pos * u_uv.zw;
  vec2 p = (u_rect.xy + a_pos * u_rect.zw) / u_view * 2.0 - 1.0;
  gl_Position = vec4(p.x, -p.y, 0.0, 1.0);
}`;

/**
 * Épreuve CMJN : simulation à l'écran d'une impression quadri selon un vrai profil de presse (celui des Paramètres
 * de couleur, ou d'un document en mode CMJN : voir lib/color-settings.svelte.ts). La conversion sRGB → CMJN → sRGB est
 * calculée par LittleCMS dans le moteur (engine::cmyk, une table 3D de N points par axe par espace CMJN) ; ici on se contente de la lire avec interpolation trilinéaire. Le pixel relu de l'écran est
 * opaque (canevas sans alpha) : pas de division à faire. Le bord des cellules est décalé d'un demi-texel pour que
 * 0 et 1 tombent exactement sur les premier et dernier points de la table.
 */
export const PROOF_SHADER = `#version 300 es
precision highp float;
precision highp sampler3D;
uniform sampler2D u_tex;
uniform sampler3D u_lut;
uniform float u_lutSize;
out vec4 o_color;
void main() {
  vec3 px = texelFetch(u_tex, ivec2(gl_FragCoord.xy), 0).rgb;
  vec3 at = (clamp(px, 0.0, 1.0) * (u_lutSize - 1.0) + 0.5) / u_lutSize;
  o_color = vec4(texture(u_lut, at).rgb, 1.0);
}`;

/**
 * Vue tournée (outil Rotation de l'affichage, R) : dessin final de l'image composée droite dans la texture du cadre
 * tourné. Pour chaque pixel du canevas (qui, lui, ne tourne pas), on cherche le point du cadre qui doit s'y trouver :
 * exactement `screenToFrame` de camera.ts (rotation inverse autour du centre de la zone de travail, puis coin du
 * cadre), en pixels CSS, ramené aux pixels physiques de la texture. Lecture bilinéaire ; hors de la partie dessinée de
 * la texture (n'arrive pas : le cadre couvre la zone de travail à tout angle), le gris du fond.
 */
export const PRESENT_SHADER = `#version 300 es
precision highp float;
uniform sampler2D u_tex;
uniform vec2 u_view;    // canevas, pixels physiques
uniform float u_dpr;    // pixels physiques du canevas par pixel CSS (résolution dynamique comprise)
uniform float u_scale;  // pixels de la texture par pixel CSS (le même, sauf texture plafonnée par la carte)
uniform vec2 u_center;  // centre de la zone de travail, pixels CSS
uniform vec2 u_frame;   // coin haut gauche du cadre tourné par rapport à la zone de travail, pixels CSS
uniform vec2 u_rot;     // cosinus et sinus de la rotation de l'affichage
uniform vec2 u_region;  // partie de la texture où l'image a été composée, pixels physiques
uniform vec2 u_texSize;
uniform vec3 u_bg;
out vec4 o_color;
void main() {
  vec2 s = vec2(gl_FragCoord.x, u_view.y - gl_FragCoord.y) / u_dpr;
  vec2 d = s - u_center;
  vec2 v = u_center + vec2(d.x * u_rot.x + d.y * u_rot.y, -d.x * u_rot.y + d.y * u_rot.x);
  vec2 f = (v - u_frame) * u_scale;
  if (f.x < 0.0 || f.y < 0.0 || f.x > u_region.x || f.y > u_region.y) {
    o_color = vec4(u_bg, 1.0);
    return;
  }
  // Composition faite avec l'origine OpenGL en bas à gauche de la partie dessinée.
  vec2 g = clamp(vec2(f.x, u_region.y - f.y), vec2(0.5), u_region - 0.5);
  o_color = vec4(texture(u_tex, g / u_texSize).rgb, 1.0);
}`;

/**
 * Mode Gris ou Bitmap (Image › Mode) sur un aperçu en couches composé par le processeur (déplacement, opacité,
 * réglage : renderer.ts `drawPreview`) : les couches du moteur ne portent pas la règle de mode, elles sont d'abord
 * assemblées dans une texture hors écran, puis ce programme pose le RÉSULTAT composé, désaturé ou seuillé (le seuil
 * bitmap porte sur l'image composée, pas sur chaque couche). `finishColor` est mot pour mot celle de gpu-layers.ts
 * (même formule entière que le moteur, `color_mode::finish_tile`) ; ui/tests/gpu-layers.test.mjs vérifie qu'elles
 * restent identiques. `u_colorMode` : 1 gris, 2 seuillé (voir `finishModeFor`, camera.ts).
 */
export const FINISH_SHADER = `#version 300 es
precision highp float;
precision highp int;
uniform sampler2D u_tex;
uniform int u_colorMode;
out vec4 o_color;
vec4 finishColor(vec4 c) {
  ivec4 q = ivec4(floor(clamp(c, 0.0, 1.0) * 255.0 + 0.5));
  int y = (77 * q.r + 150 * q.g + 29 * q.b + 128) >> 8;
  if (u_colorMode == 1) return vec4(vec3(float(y)), float(q.a)) / 255.0;
  if (q.a < 128) return vec4(0.0);
  return y * 255 >= 128 * q.a ? vec4(1.0) : vec4(0.0, 0.0, 0.0, 1.0);
}

void main() {
  o_color = finishColor(texelFetch(u_tex, ivec2(gl_FragCoord.xy), 0));
}`;
