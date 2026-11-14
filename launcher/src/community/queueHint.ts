// What the ticket card says while a player waits in a queue: pure functions (no imports, so that `npm run test:ui`
// runs them with Node alone). Everything comes from fields the server already gives: `GET /v1/queues` (`searching`,
// `teams`, `eligible`, `availability`) and the `reason` of a `queue.ticket` push. The texts live in `texts.ts` (`q.hint.*`).

// eslint-disable-next-line @typescript-eslint/no-explicit-any
type J = any;

export interface QueueHint {
  /** Players searching in the player's queue (the player included), or null when the server does not tell. */
  here: number | null;
  /** Other queues of the same format where players are searching: the players may have joined different ones. */
  others: { id: string; n: number }[];
  /** `alone` (nobody else in this queue), `others` (players wait in another queue of this format), `closed`,
   *  `requeued` (a match fell through), `waiting` (others are searching: the server pairs by rating and game). */
  reason: "closed" | "requeued" | "others" | "alone" | "waiting" | "unknown";
}

const sameFormat = (a: J, b: J) =>
  Array.isArray(a?.teams) && Array.isArray(b?.teams) && a.teams[0] === b.teams[0] && a.teams[1] === b.teams[1];

/** The hint for a ticket of `queueId`, from the queue list and the reason of the last ticket push. */
export function queueHint(queueId: string, queues: J[] | null, ticketReason?: string | null): QueueHint {
  const mine = (queues ?? []).find((q: J) => q?.id === queueId);
  const here = typeof mine?.searching === "number" ? mine.searching : null;
  const others = (queues ?? [])
    .filter((q: J) => q?.id !== queueId && typeof q?.searching === "number" && q.searching > 0 && sameFormat(q, mine))
    .map((q: J) => ({ id: String(q.id), n: q.searching as number }));
  let reason: QueueHint["reason"] = "unknown";
  if (mine?.availability?.open === false) reason = "closed";
  else if (ticketReason === "requeued") reason = "requeued";
  else if (here !== null && here <= 1) reason = others.length ? "others" : "alone";
  else if (here !== null) reason = "waiting";
  return { here, others, reason };
}
