"""What a run did to a board: the routing of the lots and the seeing to starved pads both add to it."""

from dataclasses import dataclass, field


@dataclass
class Outcome:
    status: str = "routed"  # routed, or how the engine's last incomplete run ended (unroutable, conflicts)
    wanted: int = 0  # nets that were to be joined
    left: int = 0  # of them, those still not joined
    tracks: int = 0
    vias: int = 0
    failure: object = None  # what the engine said, if a run of it failed
    tied: int = 0  # pads with too few spokes to their zone that were given a track to it
    starved: list = field(default_factory=list)  # and those left with too few, by name

    @property
    def laid(self):
        return self.tracks + self.vias > 0
