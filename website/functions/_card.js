// The gamercard: a small image of someone's profile (gamer picture,
// gamertag, Gamerscore, latest achievements) for Discord, forums and
// signatures. Built as SVG; the owner's browser turns it into the PNG
// served at /card/<gamertag>.png (see refreshCard in site.js).

const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&apos;' })[c]);
const fit = (s, n) => ([...String(s ?? '')].length > n ? [...s].slice(0, n - 1).join('') + '…' : String(s ?? ''));

export const CARD_W = 600, CARD_H = 210;

/**
 * p: {gamertag, motto, color, gamerscore, unlocked, avatarLetter}
 * pic: data URI of the gamer picture (or null to draw the letter)
 * icons: up to 5 {uri, name} for the latest achievements
 */
export function cardSvg(p, pic, icons) {
  const accent = /^#[0-9a-f]{6}$/i.test(p.color || '') ? p.color : '#4cc931';
  const tiles = icons.slice(0, 5).map((ic, i) => {
    const x = 196 + i * 62;
    return `<g><rect x="${x - 1}" y="125" width="54" height="54" rx="9" fill="#0b1117" stroke="#ffffff" stroke-opacity=".18"/>
      <clipPath id="ic${i}"><rect x="${x + 2}" y="128" width="48" height="48" rx="7"/></clipPath>
      <image href="${ic.uri}" x="${x + 2}" y="128" width="48" height="48" clip-path="url(#ic${i})" preserveAspectRatio="xMidYMid slice"/></g>`;
  }).join('');
  return `<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="${CARD_W}" height="${CARD_H}" viewBox="0 0 ${CARD_W} ${CARD_H}">
  <defs>
    <linearGradient id="bg" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#15212c"/><stop offset="1" stop-color="#0a1016"/></linearGradient>
    <linearGradient id="glow" x1="0" y1="0" x2="1" y2="0"><stop offset="0" stop-color="${accent}" stop-opacity=".55"/><stop offset=".6" stop-color="${accent}" stop-opacity="0"/></linearGradient>
    <linearGradient id="gloss" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#fff" stop-opacity=".14"/><stop offset="1" stop-color="#fff" stop-opacity="0"/></linearGradient>
    <clipPath id="card"><rect width="${CARD_W}" height="${CARD_H}" rx="18"/></clipPath>
    <clipPath id="pic"><rect x="22" y="22" width="150" height="150" rx="16"/></clipPath>
  </defs>
  <g clip-path="url(#card)">
    <rect width="${CARD_W}" height="${CARD_H}" fill="url(#bg)"/>
    <rect width="${CARD_W}" height="${CARD_H}" fill="url(#glow)" opacity=".7"/>
    <path d="M0 0 H${CARD_W} V70 C${CARD_W * 0.6} 92 ${CARD_W * 0.3} 92 0 70 z" fill="url(#gloss)"/>
    <rect x="0" y="${CARD_H - 4}" width="${CARD_W}" height="4" fill="${accent}"/>
  </g>
  ${pic
    ? `<image href="${pic}" x="22" y="22" width="150" height="150" clip-path="url(#pic)"/>`
    : `<rect x="22" y="22" width="150" height="150" rx="16" fill="${accent}"/><text x="97" y="122" text-anchor="middle" font-family="Selawik" font-weight="600" font-size="76" fill="#fff">${esc(p.avatarLetter || '?')}</text>`}
  <rect x="22" y="22" width="150" height="150" rx="16" fill="none" stroke="#fff" stroke-opacity=".3" stroke-width="2"/>
  <text x="196" y="58" font-family="Selawik" font-weight="600" font-size="32" fill="#ffffff">${esc(fit(p.gamertag, 18))}</text>
  ${p.motto ? `<text x="196" y="82" font-family="Selawik" font-size="15" fill="#9fb1c2">${esc(fit(p.motto, 46))}</text>` : ''}
  <circle cx="207" cy="${p.motto ? 106 : 92}" r="11" fill="#4cc931"/><text x="207" y="${p.motto ? 111 : 97}" text-anchor="middle" font-family="Selawik" font-weight="600" font-size="14" fill="#08200a">G</text>
  <text x="224" y="${p.motto ? 113 : 99}" font-family="Selawik" font-weight="600" font-size="21" fill="#ffffff">${Number(p.gamerscore || 0).toLocaleString('en-US')}</text>
  <text x="${CARD_W - 22}" y="${p.motto ? 113 : 99}" text-anchor="end" font-family="Selawik" font-size="14" fill="#9fb1c2">${Number(p.unlocked || 0).toLocaleString('en-US')} achievements</text>
  ${tiles || `<text x="196" y="158" font-family="Selawik" font-size="14" fill="#6f8294">No achievements yet</text>`}
  <text x="${CARD_W - 22}" y="${CARD_H - 18}" text-anchor="end" font-family="Selawik" font-weight="600" font-size="11" letter-spacing="1.5" fill="#4cc931">ACHIEVEMENT HUB</text>
</svg>`;
}
