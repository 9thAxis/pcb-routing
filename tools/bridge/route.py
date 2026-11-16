"""Joining the nets of a board: the lots they go in, and each lot through its tries."""

import math
from dataclasses import replace

from .intent import told, unheeded
from .rules import Rules
from .questions import on_zone, parts_of
from .problem import problem_of
from .engine import ENGINE_NETS, EngineFailed
from .outcome import Outcome
from .ties import tie_starved

BATCH_NETS = 200  # the nets of one lot, where a board has more than the engine takes in one run
POUR_REPEATS = 8  # how often nets that new tracks have parted from their pour are joined again, for one lot of nets
POUR_AGAINS = 3  # and how often the lot is taken back and laid again to hold a cut pour together


def batches(names, most=ENGINE_NETS, size=BATCH_NETS, together=()):
    """The nets in lots the engine accepts: all at once if fewer than `most`, else evenly in lots of at most
    `size`. `together`: pairs of nets that go in one lot (the halves of a differential pair); the second
    joins the first where they would be parted."""
    if len(names) < most:
        return [list(names)] if names else []
    count = math.ceil(len(names) / size)
    each = math.ceil(len(names) / count)
    lots = [list(names[k:k + each]) for k in range(0, len(names), each)]
    lot_of = {name: lot for lot in lots for name in lot}
    for a, b in together:
        if a in lot_of and b in lot_of and lot_of[a] is not lot_of[b]:
            lot_of[b].remove(b)
            lot_of[a].append(b)
            lot_of[b] = lot_of[a]
    return [lot for lot in lots if lot]


def route(board, settings, engine, only=None, options=None, say=print):
    """Join the unjoined nets of a board (those named in `only`, if given).

    `board` reads and writes it: `read(nets)` gives its picture as it now stands (with the groups of joined pads
    of those nets at least), `write(solution, sizes)` adds a solution's copper and returns the numbers of track
    segments and vias added, `unjoined(nets)` names those of the nets whose pads are still not all joined; for
    a board with pours, `take_back(keys)` removes what the last solution written laid for those entries,
    `mark()` gives the place in the record of what has been written and `rewind(mark)` removes all written
    since (both return the numbers of track segments and vias removed).
    `engine` runs problems and answers the bridge's questions (see `Engine`).

    The nets go in lots the engine accepts, and each lot through its tries (see `_Lot`).
    """
    run = _Run(board, settings, engine, options, say)
    outcome, wanted = run.outcome, run.wanted
    picture = board.read(only)
    names = picture.unjoined(only)
    run.open_before = set(board.unjoined())
    starved_before = {(t.pad, t.layer) for t in picture.thermals if t.spokes < t.least}
    laid_before = picture.copper_keys()
    for line in told(Rules.of(picture, settings).intents, set(names)) + unheeded(picture, settings.hints):
        say(line)
    intents = Rules.of(picture, settings).intents
    lots = batches(names, together=[(a, intents[a].pair) for a in names if intents[a].pair and a < intents[a].pair])
    if len(lots) > 1:
        say(f"{len(names)} nets to join, more than the engine takes in one run: routed in {len(lots)} batches "
            f"of at most {max(len(l) for l in lots)}, each against the board as the batch before left it")
    # (a question that gets no answer ends the run as a failed run of the engine does: what has been laid is kept)
    try:
        for number, lot in enumerate(lots):
            if number:
                picture = board.read(lot)
                say(f"batch {number + 1} of {len(lots)}")
            _Lot(run, lot, picture).join()
            if outcome.failure:
                break
            run.given_up = set(board.unjoined(set(wanted))) if wanted else set()
        if settings.pours and not outcome.failure:
            tie_starved(board, settings, engine, outcome, only, starved_before, options, say, laid_before)
    except EngineFailed as failure:
        outcome.failure = str(failure)
    # (a net that was joined and that the new tracks parted for good is one left unjoined, as much as any)
    try:
        wanted += [n for n in board.unjoined() if n not in run.open_before and n not in wanted] if wanted else []
        # the board has the last word: a net is joined when its pads are
        outcome.left = len(board.unjoined(set(wanted))) if wanted else 0
    except EngineFailed as failure:  # (what the zones now join is not known: every net counts as left)
        outcome.failure, outcome.left = outcome.failure or str(failure), len(wanted)
    outcome.wanted = len(wanted)
    outcome.status = "routed" if outcome.left == 0 else run.last if run.last != "routed" else "unroutable"
    return outcome


class _Run:
    """What `route` carries from one lot of nets to the next."""

    def __init__(self, board, settings, engine, options, say):
        self.board, self.settings, self.engine, self.options, self.say = board, settings, engine, options, say
        self.outcome = Outcome()
        self.wanted = []  # the nets that were to be joined, in the order met
        self.last = "routed"  # how the engine's last run of a lot's last try ended
        self.first = True  # no run of the engine has been kept yet
        self.open_before = set()  # nets that are not this run's to join, or were not joined to begin with
        self.given_up = set()  # nets that an earlier lot left unjoined
        self.noted = set()  # what has been said once and is not said again


class _Lot:
    """One lot of nets on its way through its tries (see Rules.tries): each try is one run of the engine for
    what the try before left unjoined, against the board as it then stands. A run can be done over before
    anything of it is written:

    - without the links of capacitors to their pins, for a net that was not all laid with them;
    - with a pair begun further from its pads, and in the end as two nets, where the pair was not all laid;
      and, where something else is left unjoined, once without any pair and without what was only guessed
      of the nets, of which the one that joins more is kept.

    And after it is written, for the zones:

    - nets that the new tracks parted from their zone are joined again, in runs of their own;
    - where that cannot be done past the tracks that did it, everything written for the lot is taken back
      and the lot is laid again with those pads joined by tracks.
    """

    def __init__(self, run, lot, picture):
        self.run, self.lot, self.picture = run, lot, picture
        self.tries = Rules.of(picture, run.settings).tries(lot)
        self.number, self.reread = 0, False  # the try at hand; whether the board is to be read again for it
        # the nets given up as linked; the pairs not all laid as pairs, with how far off their pads they are
        # begun next (see pair_entries)
        self.unlinked, self.unpaired = set(), {}
        # a run with wishes granted (pairs, what the names were taken to ask) kept to be weighed against one
        # without; whether that was done; and whether the wishes are off for the rest of the lot
        self.weighing, self.weighed, self.plain = None, False, False
        # For pours that new tracks cut: how often their nets have been joined again; the last run of the lot's
        # own nets, as (its problem, where the board's record stood before it, its picture); whether the present
        # run is one that joins a cut pour again; the entries that hold cut pours together; whether the lot's
        # run is being done again with those, and how often it has been.
        self.repeats, self.base, self.rejoining, self.bridges, self.again, self.agains = 0, None, False, [], False, 0

    def join(self):
        run = self.run
        while self.number < len(self.tries):
            problem = self.drawn_up()
            if problem is None:
                continue  # (no try at this step)
            # (a net may have several entries; the trunk of a pair is of no net of the board)
            keys = list(dict.fromkeys(n["net"] for n in problem["nets"] if n["net"] in self.picture.groups))
            if not self.number and self.reread is False and not self.unlinked and not self.unpaired:
                run.wanted += [n for n in keys if n not in run.wanted]
            if not keys:
                break
            if self.number and self.reread:
                step = self.tries[self.number][0]
                finer = step != self.tries[self.number - 1][0]
                run.say(f"{'next try, at a %g mm step' % step if finer else 'second try, narrower'}: {', '.join(keys)}")
            try:
                solution = run.engine.route(problem, run.first)
            except EngineFailed as failure:
                run.outcome.failure = str(failure)
                break
            kept = self.without_wishes(problem, solution)
            if kept is None:
                continue  # (done over)
            problem, solution = kept
            run.first = False
            for pair in problem.get("pairs", []):
                run.say(f"laid as a pair: {pair['key']}")
            mark = self.written(problem, solution)
            if solution["status"] == "routed":
                if not self.joined_again(problem, mark):
                    break
            elif not self.held_together(problem, keys):
                # (the next try is for what is left, against the board as it stands: nothing of this one is laid again)
                self.base, self.rejoining, self.bridges, self.again = None, False, [], False
                run.last = solution["status"]
                self.lot = keys
                self.number, self.reread = self.number + 1, True
                self.unpaired, self.weighed = {}, False  # (on a finer grid a pair has its chance again)

    def at_step(self):
        """The settings of the try at hand. (No pairs among nets that are only being joined again after a pour
        was cut.)"""
        settings = self.run.settings
        return replace(settings, step=self.tries[self.number][0],
                       pairs=settings.pairs and not self.rejoining and not self.plain,
                       lean=settings.lean and not self.plain)

    def drawn_up(self):
        """The problem of the next run; None if the try at hand is passed over (its grid would be too large)."""
        run, (step, narrow) = self.run, self.tries[self.number]
        if self.again:
            self.picture = self.base[2]
            return dict(self.base[0], nets=[n for n in self.base[0]["nets"] if " ~ " not in n["key"]] + self.bridges)
        if self.reread:
            # what could not be joined is tried once more against the board as it now stands
            self.picture = run.board.read(self.lot)
        rules = Rules.of(self.picture, self.at_step())
        most = (run.options or {}).get("max_states")
        if self.number and most and rules.states(self.picture.box, self.picture.layers) > most:
            run.say(f"no try at a {step:g} mm step: the grid would be larger than --max-states")
            self.number, self.reread = self.number + 1, True
            return None
        notes = []
        problem = problem_of(self.picture, rules, run.engine.ask, only=set(self.lot), narrow=narrow,
                             options=run.options, unlinked=self.unlinked, unpaired=self.unpaired, notes=notes)
        for note in notes:
            if note not in run.noted:
                run.say(note)
        run.noted.update(notes)
        return problem

    def without_wishes(self, problem, solution):
        """What of a run is kept, as (problem, solution); None if it is to be done over without a wish that
        was not granted: nothing of it is written then."""
        run, narrow = self.run, self.tries[self.number][1]
        # A link from a capacitor to its pin is a wish, not a need. Where a net with such links is not all
        # laid, the run is done again with that net whole, as it is from then on.
        failed = {n["net"] for n in problem["nets"]
                  if any(m.startswith(n["key"] + ":") for m in solution.get("unrouted", []))}
        failed |= {n["net"] for n in problem["nets"] for c in solution.get("conflicts", []) if c.get("key") == n["key"]}
        linked = {n["net"] for n in problem["nets"] if n["key"].startswith(n["net"] + " | ")} - self.unlinked
        # So is laying two nets as a pair. First the links are given up, and the pairs stay.
        if failed & linked and not self.again:
            run.say("not all joined with the capacitors linked to their pins: again without those links for "
                    f"{', '.join(sorted(failed & linked))}")
            self.unlinked |= failed & linked
            self.reread = False
            return None
        # Then a pair that is not all laid is done again begun further from its pads (so near them its stubs
        # may have no room), and as two nets when it has been begun as far off as it is offered; and where
        # something else is left unjoined, the run is done once more without any pair (they take room) and
        # without what was only guessed of the nets (a clock kept off vias takes room too): that run stands
        # if it joins more, for the rest of the lot; else the first.
        said = solution.get("unrouted", []) + [c.get("key", "") + ":" for c in solution.get("conflicts", [])]
        leaned = Rules.of(self.picture, self.at_step()).leans(n["net"] for n in problem["nets"])
        of_pair = lambda pair: [pair["key"], pair["trunk"]] + [stub for end in pair["stubs"] for stub in end]
        broken = {pair["key"] for pair in problem.get("pairs", [])
                  if any(m.startswith(key + ":") for m in said for key in of_pair(pair))}
        if self.weighing is not None:
            if len(said) >= self.weighing[2]:
                run.say("no more is joined without: they are kept")
                (problem, solution, _), self.plain = self.weighing, False
            self.weighing, broken = None, set()
        elif broken:
            run.say("not all laid as pairs: again, begun further from their pads or as single nets, for "
                    f"{', '.join(sorted(broken))}")
        elif said and not self.weighed and not self.again and (problem.get("pairs") or leaned):
            wishes = (["pairs"] if problem.get("pairs")
                      else []) + (["what the nets' names were taken to ask"] if leaned else [])
            run.say(f"not all joined: once more without {' and '.join(wishes)}")
            self.weighing, self.weighed, self.plain = (problem, solution, len(said)), True, True
            self.reread = False
            return None
        if not broken:
            return problem, solution
        self.unpaired.update({key: self.unpaired.get(key, 0) + 1 for key in broken})
        if self.again:
            # (a lot that is being laid again to hold a cut pour together: its problem is drawn up anew)
            base, mark, picture = self.base
            of_lot = {n["net"] for n in base["nets"] if n["net"] in picture.groups}
            anew = problem_of(picture, Rules.of(picture, self.at_step()), run.engine.ask, only=of_lot, narrow=narrow,
                              options=run.options, unlinked=self.unlinked, unpaired=self.unpaired)
            self.base = (anew, mark, picture)
        self.reread = False
        return None

    def written(self, problem, solution):
        """Write what a run laid; returns where the board's record stood before it. A track that ends on a
        pour is joined only while the pour is there: where the other new tracks have cut it away, the copper
        that hangs on that end is taken back, and its net is among those still to join."""
        run, board = self.run, self.run.board
        mark = board.mark() if run.settings.pours else None
        # (an entry that has points of a pour among its terminals is written piece by piece)
        if run.settings.pours:
            parts = parts_of(self.picture, problem, solution, run.engine.ask)
        else:
            parts = [dict(laid, of=laid["key"]) for laid in solution["nets"]]
        sizes = {n["key"]: (n["width"], n["via"]) for n in problem["nets"]}
        # (what is laid for a pair comes back per leg, under the pair's key and the leg's net: the leg and its stubs)
        sizes.update({f"{pair['key']}/{leg}": sizes[stub] for pair in problem.get("pairs", [])
                      for leg, stub in zip(pair["legs"], pair["stubs"][0])})
        added = board.write(dict(solution, nets=parts), {part["key"]: sizes[part["of"]] for part in parts})
        ends = [(part["key"], end) for part in parts for end in part.get("ends", [])]
        standing = on_zone(board.read(), [end for _, end in ends], run.engine.ask) if ends else []
        loose = sorted({key for (key, _), stands in zip(ends, standing) if not stands})
        if loose:
            taken = board.take_back(loose)
            added = (added[0] - taken[0], added[1] - taken[1])
            nets_of = {part["key"]: part["net"] for part in parts}
            run.say("ended on a pour that the other new tracks cut away, taken back: "
                    f"{', '.join(sorted({nets_of[key] for key in loose}))}")
        run.outcome.tracks, run.outcome.vias = run.outcome.tracks + added[0], run.outcome.vias + added[1]
        return mark

    def joined_again(self, problem, mark):
        """After a run that laid all it was given: True if another is to follow, for nets the new tracks have
        parted (they may have cut a pour and parted pads it joined: those nets are joined again, against the
        board as it now stands); False if the lot is done."""
        run = self.run
        # (any net of this lot so far, or one that was joined before the run began)
        mine = lambda n: (n in run.wanted or n not in run.open_before) and n not in run.given_up
        cut = [n for n in run.board.unjoined() if mine(n)] if self.repeats < POUR_REPEATS else []
        if not cut:
            return False
        if not self.rejoining:
            self.base = (problem, mark, self.picture)
        run.say(f"parted by the new tracks, joined again: {', '.join(cut)}")
        run.wanted += [n for n in cut if n not in run.wanted]
        self.lot, self.reread, self.repeats, self.rejoining, self.again = cut, True, self.repeats + 1, True, False
        return True

    def held_together(self, problem, keys):
        """After a run that left something unjoined: True if the lot is to be laid again from the start to hold
        a cut pour together.

        A cut pour could not be joined again past the tracks that cut it (they may have shut a piece of it
        in). Everything laid since is taken back, and the lot is laid again with the pads of the pieces as
        they were cut to be joined by a track, pad to pad: the others then leave the way for it. (Not from a
        point of the pour to another: on the pour whole again those are a step apart, and a track between
        them holds nothing together.)"""
        run, narrow = self.run, self.tries[self.number][1]
        entries = len(self.base[0]["nets"]) + len(self.bridges) + len(problem["nets"]) if self.base else 0
        room = self.base is not None and entries < ENGINE_NETS
        if not (self.rejoining and room and run.settings.pours and self.agains < POUR_AGAINS):
            return False
        taken = run.board.rewind(self.base[1])
        run.outcome.tracks, run.outcome.vias = run.outcome.tracks - taken[0], run.outcome.vias - taken[1]
        by_pads = problem_of(self.picture, Rules.of(self.picture, replace(self.at_step(), pours=False, pairs=False)),
                             run.engine.ask, only=set(keys), narrow=narrow, options=run.options, unlinked=set(keys))
        self.bridges += [dict(entry, key=f"{entry['net']} ~ {len(self.bridges) + k + 1}")
                         for k, entry in enumerate(by_pads["nets"])]
        run.say("no way to join them past the new tracks: those are laid again, with the pieces of the pour held "
                f"together: {', '.join(keys)}")
        self.lot = list(dict.fromkeys(n["net"] for n in self.base[0]["nets"]))
        self.reread, self.rejoining, self.again, self.agains = False, False, True, self.agains + 1
        return True
