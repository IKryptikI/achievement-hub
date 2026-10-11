// Keeps hateful and crude words off the site.
//
// Slurs are refused everywhere people write (gamertags, mottos, bios,
// forum posts, tips, requests). General profanity and staff-sounding names
// are refused in gamertags only (they show up everywhere, including other
// people's overlays).
//
// Text is normalized first, so l33t-speak, spacing, punctuation and
// stretched letters don't get around it: "N 1 g g 3 r", "n.i.g.g.a",
// "niiiigga" all read as the same word.

const LEET = { 0: 'o', 1: 'i', 2: 'z', 3: 'e', 4: 'a', 5: 's', 6: 'g', 7: 't', 8: 'b', 9: 'g', '@': 'a', $: 's', '!': 'i', '|': 'i', '+': 't', '€': 'e', '¡': 'i' };

/** Lowercase letters only, look-alikes mapped, accents dropped. */
function letters(text) {
  return String(text ?? '')
    .normalize('NFKD')
    .replace(/[̀-ͯ]/g, '')
    .toLowerCase()
    .replace(/[0-9@$!|+€¡]/g, (c) => LEET[c] || c)
    .replace(/[^a-z]/g, '');
}
/** ...and repeated letters collapsed ("niiigggaa" -> "niga"). */
const squeeze = (s) => s.replace(/(.)\1+/g, '$1');

// Matched anywhere inside the normalized text (after squeezing). These
// strings don't occur inside ordinary words.
const SLURS = [
  'niger', 'nigga', 'nigah', 'nigar', 'nigur', 'niga', 'nigr', 'nikker', 'niker', 'nibba', 'negro', 'negroid',
  'kike', 'kyke', 'chink', 'gook', 'wetback', 'beaner', 'spick', 'raghead', 'towelhead', 'sandnig', 'porchmonk',
  'junglebunn', 'coonass', 'golliwog', 'faget', 'fagot', 'fagit', 'tranny', 'trany', 'shemale', 'retard',
];
// Real words that happen to contain one of those.
const SLUR_INNOCENT = ['snigger', 'sniger', 'nigeria', 'denigrat', 'negroni', 'montenegr', 'zuniga', 'retardant'];
// Short slurs that are also pieces of normal words: whole words in
// sentences; anywhere in a gamertag, unless an innocent word explains it.
const SHORT_SLURS = ['fag', 'coon', 'spic', 'spick', 'spik', 'jap', 'paki', 'wop', 'chink', 'dyke'];
const INNOCENT = [
  'raccoon', 'racoon', 'cocoon', 'tycoon', 'coonhound', 'spice', 'spicy', 'conspic', 'auspic', 'perspic', 'spicule', 'japan', 'jape',
  'pakistan', 'swop', 'wopper', 'faggio', 'vandyke',
];
// Crude words: kept out of gamertags only.
const CRUDE = [
  'fuck', 'fuk', 'fck', 'shit', 'cunt', 'bitch', 'bich', 'whore', 'slut', 'cock', 'dick', 'pussy', 'puss', 'penis', 'vagina',
  'porn', 'rape', 'rapist', 'molest', 'pedo', 'paedo', 'nazi', 'hitler', 'heil', 'kkk', 'anal', 'anus', 'tits', 'boob', 'jizz',
  'cum', 'semen', 'dildo', 'bastard', 'asshole', 'arsehole', 'twat', 'wank', 'nonce', 'incest', 'isis', 'jihad',
];
const CRUDE_INNOCENT = [
  'peacock', 'cockpit', 'cocktail', 'cockatoo', 'cockroach', 'hancock', 'babcock', 'dickens', 'grape', 'drape', 'scrape', 'trapeze',
  'rapeseed', 'therap', 'parapet', 'canal', 'banal', 'analy', 'analo', 'cucumber', 'circum', 'document', 'cumul', 'cumber', 'incum',
  'succumb', 'cumin', 'titan', 'titl', 'stitch', 'shitake', 'shiitake', 'pussycat', 'pedal', 'speedo', 'torpedo', 'encyclopedi',
  'basement', 'casement', 'scunthorpe', 'janus', 'manus', 'swank', 'fukuo', 'fukush', 'bichon', 'basis', 'crisis', 'oasis',
];
// Names that look official.
const RESERVED = ['admin', 'administrator', 'moderator', 'mod', 'staff', 'support', 'official', 'achievementhub', 'achievhub', 'developer', 'dev', 'system', 'root', 'owner', 'anthropic', 'claude'];

/** `text` with every innocent word blanked out (so its letters can't form a match). */
function withoutInnocent(s, words) {
  for (const w of words) s = s.split(w).join('_');
  return s;
}

function hasSlur(text, { anywhere }) {
  const raw = withoutInnocent(letters(text), SLUR_INNOCENT);
  const flat = withoutInnocent(squeeze(letters(text)), SLUR_INNOCENT);
  if (SLURS.some((w) => raw.includes(w) || flat.includes(w))) return true;
  if (anywhere) {
    const clean = withoutInnocent(flat, INNOCENT);
    return SHORT_SLURS.some((w) => clean.includes(w));
  }
  // In sentences: short slurs only as whole words (spacing tricks aside).
  const words = String(text ?? '').toLowerCase().split(/[^a-z0-9@$!|+]+/).map((w) => squeeze(letters(w)));
  return words.some((w) => SHORT_SLURS.includes(w) || SHORT_SLURS.some((s) => w === s + 's'));
}

/** Why a gamertag can't be used, or null if it's fine. */
export function badGamertag(tag) {
  if (hasSlur(tag, { anywhere: true })) return "That gamertag isn't allowed.";
  const raw = withoutInnocent(letters(tag), CRUDE_INNOCENT);
  const flat = withoutInnocent(squeeze(letters(tag)), CRUDE_INNOCENT);
  // (Squeezed words only when they stay words: "kkk" squeezes to "k".)
  if (CRUDE.some((w) => raw.includes(w) || (squeeze(w).length >= 3 && flat.includes(squeeze(w))))) return "That gamertag isn't allowed. Keep it clean!";
  if (RESERVED.includes(letters(tag)) || RESERVED.includes(flat)) return 'That gamertag is reserved.';
  return null;
}

/** Why some text can't be posted, or null if it's fine. */
export function badText(...texts) {
  return texts.some((t) => hasSlur(t, { anywhere: false })) ? "Please keep slurs and hate speech off Achievement Hub." : null;
}
