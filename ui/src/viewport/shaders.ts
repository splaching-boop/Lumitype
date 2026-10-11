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
 * Épreuve CMJN : simulation à l'écran d'une impression quadri selon un vrai profil de presse (FOGRA39, FOGRA51,
 * FOGRA52, GRACoL, SWOP : une table par profil, voir lib/proof-profiles.ts). La conversion sRGB → CMJN → sRGB est précalculée par LittleCMS dans une table 3D (`export/lut-cmjn.py`,
 * N points par axe) ; ici on se contente de la lire avec interpolation trilinéaire. Le pixel relu de l'écran est
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
