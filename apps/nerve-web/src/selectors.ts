/**
 * What a selector matched, and what it passed over — the pure half.
 *
 * One path can have two readings. `src/app.ts` holds both a `Module` and a `File`;
 * `docs/architecture.md` holds both a `Document` and a `File`. The server's rule is **content
 * wins, container is reported**, and before Slice 8b-i the container was passed over with no
 * indication that a choice had been made at all — on the documentation fixture that was 26 % of
 * entities unreachable by their most natural identifier.
 *
 * The server has reported the choice on every selector answer since. Nothing rendered it, which
 * made the report worth exactly as much as not making it: a reader typed one path, was shown one
 * entity, and had no way to learn that a second thing lives there. This file is the reading half,
 * and it is separate from the components for the usual reason — the sentence a reader gets is a
 * decision, and a decision inline in JSX is a decision no test can see.
 *
 * **Nothing here chooses.** The server chose, by a stated rule; these functions name the choice and
 * name the alternative. Where a selector was ambiguous the server refuses with a `409` and every
 * candidate attached, and that is a different screen — `ui/parts.tsx`'s `Failure` — because a
 * refusal to choose and a choice made by rule are not the same event and must not read alike.
 */

import type { Entity, SelectorNotes } from './api/types';

/**
 * The entities one selector also named and a rule passed over.
 *
 * Absent and empty are read the same way, and that is sound rather than lax: `selectors` is an
 * addition to the wire shape, so a server that predates it omits the key, and *no key* and *no
 * alternatives* both mean no second reading was reported. There is no third state to lose.
 */
export function alternativesFor(selectors: SelectorNotes | undefined, key: string): Entity[] {
  const note = selectors?.[key];
  if (note === undefined) return [];
  return Array.isArray(note.alternatives) ? note.alternatives : [];
}

/** Which stage of the selector grammar matched, or `null` when the server did not say. */
export function matchedBy(selectors: SelectorNotes | undefined, key: string): string | null {
  return selectors?.[key]?.matched_by ?? null;
}

/**
 * How to type a passed-over entity back in, so the reader can go and look at it.
 *
 * A qualifier plus the path, because that is exactly what disambiguates the two readings: the
 * whole reason `file:src/app.ts` exists is that `src/app.ts` on its own resolves to the module.
 * Qualifiers are generated from the entity-kind vocabulary on the server, so `kind` **is** the
 * qualifier for every kind there is, and no list is mirrored here to fall out of date.
 *
 * The entity id is the fallback rather than the first choice. It always resolves and it is never
 * what a person would type; where there is no path there is also no ambiguity to resolve, so the
 * id is the only honest thing left to offer.
 */
export function selectorFor(entity: Pick<Entity, 'kind' | 'file_path' | 'entity_id'>): string {
  if (!entity.file_path) return entity.entity_id;
  return `${entity.kind}:${entity.file_path}`;
}

/**
 * The sentence that says a choice was made, in the reader's terms.
 *
 * Returns `''` when there is nothing to report, so a caller renders nothing rather than a panel
 * saying that nothing happened. This is the ordinary case by a wide margin — `alternatives` is
 * empty for the overwhelming majority of selectors — and a permanent "no alternatives" strip would
 * be noise on every screen in the app in order to be information on almost none.
 *
 * The wording never says the other reading is wrong, because it is not: both entities are at that
 * path and both are indexed. It says which one answered.
 */
export function passedOverReading(kind: string, alternatives: Entity[]): string {
  if (alternatives.length === 0) return '';
  const others = alternatives.length === 1 ? 'one other entity' : `${alternatives.length} other entities`;
  return `That path names ${others} as well. The ${kind} answered because content wins over the container that holds it; the other is indexed and can be asked for by name.`;
}

/**
 * Reading for the stage that matched, where it changes how firm the answer is.
 *
 * Only two of the four stages get a sentence, and the omission is the point. `entity_id` and
 * `path_qualified` are exact — the caller named one thing and got it — so a note beside them would
 * be decoration. `name` is the one worth flagging: a bare name matched one entity **this time**,
 * and the same string against a repository with a second declaration of it would be refused as
 * ambiguous rather than answered.
 */
export function matchedByReading(stage: string | null): string {
  switch (stage) {
    case 'name':
      return 'Matched on a bare name. It resolved to exactly one entity here; a repository with a second declaration of that name would refuse it rather than pick.';
    case 'path':
      return 'Matched on a repository path, which names whatever is actually at it.';
  }
  return '';
}
