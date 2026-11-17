"""KiCad: the only code that uses pcbnew."""

import json
from pathlib import Path

from .lengths import r6
from .picture import Area, Minima, NetClass, Pad, Part, Picture, Polygon, Pour, Thermal, Track, Via
from .questions import survey

try:
    import pcbnew
except ImportError:  # everything else is worked out without KiCad: only reading and writing a board need it
    pcbnew = None

ARC_ERROR = 0.002  # mm: how far the corners of a polygon drawn for a round shape lie outside it


def kicad_is_here():
    """True if this Python has KiCad's `pcbnew` module."""
    return pcbnew is not None


def ignored_checks(board_path):
    """The design rule checks the board's project sets to 'ignore' (KiCad keeps them in the .kicad_pro)."""
    try:
        project = json.loads(Path(board_path).with_suffix(".kicad_pro").read_text())
        severities = project["board"]["design_settings"]["rule_severities"]
        return frozenset(name for name, level in severities.items() if level == "ignore")
    except Exception:
        return frozenset()


def load_board(path):
    """A board file as a pcbnew board. (The file must be there: pcbnew crashes on one that is not.)"""
    return pcbnew.LoadBoard(str(path))


def save_board(board, path, fill=True):
    """Save a pcbnew board, its zones filled again first unless told not to."""
    if fill:
        pcbnew.ZONE_FILLER(board).Fill(board.Zones())
    pcbnew.SaveBoard(str(path), board)


def MM(value):
    return pcbnew.ToMM(value)


def xy(p):
    return [r6(MM(p.x)), r6(MM(p.y))]


def rings_of(poly_set):
    """Every outline of a SHAPE_POLY_SET as its rings: the outline first, then its holes."""
    out = []
    for i in range(poly_set.OutlineCount()):
        chain = poly_set.Outline(i)
        rings = [[xy(chain.CPoint(k)) for k in range(chain.PointCount())]]
        for h in range(poly_set.HoleCount(i)):
            hole = poly_set.Hole(i, h)
            rings.append([xy(hole.CPoint(k)) for k in range(hole.PointCount())])
        out.append(rings)
    return out


def pad_polygon(pad, layer):
    """A pad's copper on one of its layers, as a polygon that holds all of it: a round edge becomes corners
    that lie outside the true shape, by at most ARC_ERROR, so that what keeps its distance from the polygon
    keeps it from the pad."""
    shape = pcbnew.SHAPE_POLY_SET()
    pad.TransformShapeToPolygon(shape, layer, 0, pcbnew.FromMM(ARC_ERROR), pcbnew.ERROR_OUTSIDE)
    return shape


def pad_outline(pad, layer):
    """A pad's outline as it would be on a layer, for where it has an opening but no copper."""
    return pad_polygon(pad, layer)


def mask_opening(pad, layer):
    """How far the solder mask opening of a pad reaches beyond its outline on an outer copper layer, in mm; None
    where the mask covers it there (or on an inner layer)."""
    mask = pcbnew.F_Mask if layer == pcbnew.F_Cu else pcbnew.B_Mask if layer == pcbnew.B_Cu else None
    if mask is None or not pad.IsOnLayer(mask):
        return None
    try:
        return max(MM(pad.GetSolderMaskExpansion(layer)), 0.0)
    except TypeError:
        return max(MM(pad.GetSolderMaskExpansion()), 0.0)


def local_clearance(pad):
    """A pad's own clearance in mm, where it or its footprint overrides the net class; else None."""
    for owner in (pad, pad.GetParentFootprint()):
        try:
            value = owner.GetLocalClearance()
        except Exception:
            value = None
        if isinstance(value, tuple):
            value = value[0] if value else None
        if value:
            return MM(value)
    return None


def pad_middle(pad, layer):
    """The middle of a pad's copper: where a track meets a pad that has no hole."""
    try:
        return pad.ShapePos(layer)
    except TypeError:
        return pad.ShapePos()
    except Exception:
        return pad.GetPosition()


def pad_smallest(pad, layers):
    """A pad's smallest dimension on the given copper layers, in mm."""
    def size(layer):
        try:
            s = pad.GetSize(layer)
        except TypeError:
            s = pad.GetSize()
        return min(MM(s.x), MM(s.y))
    return min((size(l) for l in layers), default=None)


def drawing_polygon(item, layer):
    """A drawing or text on a copper layer as polygons; its bounding box if KiCad cannot give its outline."""
    poly = pcbnew.SHAPE_POLY_SET()
    try:
        item.TransformShapeToPolygon(poly, layer, 0, pcbnew.FromMM(ARC_ERROR), pcbnew.ERROR_OUTSIDE)
    except Exception:
        poly = pcbnew.SHAPE_POLY_SET()
    if poly.OutlineCount() == 0:
        box = item.GetBoundingBox()
        poly.NewOutline()
        left, right, top, bottom = box.GetLeft(), box.GetRight(), box.GetTop(), box.GetBottom()
        for x, y in ((left, top), (right, top), (right, bottom), (left, bottom)):
            poly.Append(x, y)
    return poly


def copper_layers(board):
    return [l for l in board.GetEnabledLayers().CuStack()]


def joined_pads(connectivity, pad):
    """The pads that copper already joins to a pad, followed from item to item through tracks and vias; and
    those tracks and vias."""
    found, pads, others, waiting = {pad.m_Uuid.AsString()}, [pad], [], [pad]
    while waiting:
        item = waiting.pop()
        for other in list(connectivity.GetConnectedTracks(item)) + list(connectivity.GetConnectedPads(item)):
            key = other.m_Uuid.AsString()
            if key in found:
                continue
            found.add(key)
            waiting.append(other)
            (pads if other.Type() == pcbnew.PCB_PAD_T else others).append(other)
    return pads, others


def pad_groups(board, nets=None, copper=None):
    """For each net with pads on copper (those named, if given): its groups of pads already joined. `copper`,
    if given, is filled with the tracks and vias of each group: {net: {item's id: index of its group}}."""
    enabled = copper_layers(board)
    connectivity = board.GetConnectivity()
    by_net = {}
    for pad in board.GetPads():
        name = pad.GetNetname()
        if name and pad.GetNetCode() > 0 and (nets is None or name in nets) and any(pad.IsOnLayer(l) for l in enabled):
            by_net.setdefault(name, []).append(pad)
    out = {}
    for name, pads in by_net.items():
        seen, groups = set(), []
        for pad in pads:
            if pad.m_Uuid.AsString() in seen:
                continue
            group, others = joined_pads(connectivity, pad)
            seen.update(p.m_Uuid.AsString() for p in group)
            if copper is not None:
                copper.setdefault(name, {}).update((o.m_Uuid.AsString(), len(groups)) for o in others)
            groups.append(group)
        out[name] = groups
    return out


def filled_zones(board):
    """The zones that are filled with copper of a net, each with its number among them."""
    return list(enumerate(zone for zone in board.Zones() if not zone.GetIsRuleArea() and zone.GetNetname()))


def zone_fills(board):
    """Every piece of every filled zone on copper: [Pour]. The zones must be filled."""
    name = board.GetLayerName
    out = []
    for number, zone in filled_zones(board):
        for l in copper_layers(board):
            if zone.IsOnLayer(l):
                fill = zone.GetFilledPolysList(l)
                out.extend(Pour(zone.GetNetname(), name(l), rings_of(pcbnew.SHAPE_POLY_SET(fill.Polygon(i)))[0], number)
                           for i in range(fill.OutlineCount()))
    return out


def zone_wires(board, of):
    """The vias and tracks of the nets that have zones, as the engine's survey takes them: net, layer (none
    for a via), ends, and the group of pads each is joined to (`of`: the group of each track and via, see
    pad_groups), none for copper that reaches no pad."""
    name = board.GetLayerName
    zoned = {zone.GetNetname() for _, zone in filled_zones(board)}
    out = []
    for item in board.GetTracks():
        net = item.GetNetname()
        if net in zoned:
            via = item.Type() == pcbnew.PCB_VIA_T
            out.append(dict(net=net, layer=None if via else name(item.GetLayer()),
                            group=of.get(net, {}).get(item.m_Uuid.AsString()),
                            ends=[xy(item.GetPosition())] if via else [xy(item.GetStart()), xy(item.GetEnd())]))
    return out


def zone_connection(zone, pad):
    """How a zone is to meet a pad of its net: the pad's own setting, else its footprint's, else the zone's."""
    how = pad.GetLocalZoneConnection()
    footprint = pad.GetParentFootprint()
    if how == pcbnew.ZONE_CONNECTION_INHERITED and footprint is not None:
        how = footprint.GetLocalZoneConnection()
    if how == pcbnew.ZONE_CONNECTION_INHERITED:
        how = zone.GetPadConnection()
    if how == pcbnew.ZONE_CONNECTION_THT_THERMAL:
        through = pad.GetAttribute() == pcbnew.PAD_ATTRIB_PTH
        how = pcbnew.ZONE_CONNECTION_THERMAL if through else pcbnew.ZONE_CONNECTION_FULL
    return how


def thermal_gap(zone, pad):
    """The gap a zone leaves round a pad it meets by spokes: the pad's own, else the zone's."""
    own = pad.GetLocalThermalGapOverride()
    return own if isinstance(own, int) and own > 0 else zone.GetThermalReliefGap()


def spoked_pads(board, index):
    """The pads that filled zones of their net are to meet by thermal spokes, as the engine's survey takes
    them: per pad and layer, the zone (its number, see filled_zones), the line a spoke crosses (the pad's
    outline grown by half the gap), the gap, the clearance the zone keeps, the spokes the board asks for, and
    the tracks that begin on the pad, on that layer and elsewhere. `index`: a pad's id -> its index in the picture. The
    zones must be filled and the connectivity up to date."""
    enabled = copper_layers(board)
    name = board.GetLayerName
    connectivity = board.GetConnectivity()
    least = getattr(board.GetDesignSettings(), "m_MinResolvedSpokes", 2)
    pads = {}
    for pad in board.GetPads():
        pads.setdefault(pad.GetNetname(), []).append(pad)
    out = []
    for number, zone in filled_zones(board) if least > 0 else []:
        clearance = zone.GetLocalClearance()
        clearance = max(clearance if isinstance(clearance, int) else 0, zone.GetNet().GetNetClassSlow().GetClearance())
        for l in enabled:
            fill = zone.GetFilledPolysList(l) if zone.IsOnLayer(l) else None
            if fill is None or fill.OutlineCount() == 0:
                continue
            whole = fill.BBox()
            for pad in pads.get(zone.GetNetname(), []):
                if not (pad.IsOnLayer(l)
                        and pad.FlashLayer(l)) or zone_connection(zone, pad) != pcbnew.ZONE_CONNECTION_THERMAL:
                    continue
                gap = thermal_gap(zone, pad)
                if gap <= 0 or pad.m_Uuid.AsString() not in index or not whole.Intersects(pad.GetBoundingBox()):
                    continue
                tracks = [track for track in connectivity.GetConnectedTracks(pad) if track.Type() != pcbnew.PCB_VIA_T]
                ends = lambda here: [[xy(track.GetStart()), xy(track.GetEnd())] for track in tracks
                                     if (track.GetLayer() == l) == here]
                out.append(dict(pad=index[pad.m_Uuid.AsString()], layer=name(l), zone=number, gap=MM(gap),
                                clearance=MM(clearance),
                                least=least, line=rings_of(_transformed(pad, l, gap // 2))[0][0], tracks=ends(True),
                                elsewhere=ends(False)))
    return out


def _transformed(pad, layer, by):
    """A pad's copper on a layer grown by `by`, as a polygon set."""
    out = pcbnew.SHAPE_POLY_SET()
    pad.TransformShapeToPolygon(out, layer, by, pcbnew.FromMM(ARC_ERROR), pcbnew.ERROR_OUTSIDE)
    return out


def unjoined_nets(board, nets=None):
    """The nets (of those named, if given) whose pads the board's tracks and vias do not all join, whatever
    its zones join. The board's connectivity must be up to date (BuildConnectivity after adding copper)."""
    return sorted(name for name, groups in pad_groups(board, nets).items() if len(groups) > 1)


def read_board(board, ignored, ask, pours=True):
    """A board as a Picture. With `pours`, what a filled zone joins counts as joined, and the picture has the
    zones and the pads they tie (the zones must be filled; `ask` puts the question to the engine); without,
    only tracks, vias and pads that touch join."""
    settings = board.GetDesignSettings()
    enabled = copper_layers(board)
    name = board.GetLayerName
    box = board.GetBoardEdgesBoundingBox()
    if box.GetWidth() == 0 or box.GetHeight() == 0:
        box = board.ComputeBoundingBox(False)
    picture = Picture(layers=[name(l) for l in enabled], ignored=frozenset(ignored),
                      box=[MM(box.GetLeft()), MM(box.GetTop()), MM(box.GetRight()), MM(box.GetBottom())],
                      minima=Minima(clearance=MM(settings.m_MinClearance), hole_clearance=MM(settings.m_HoleClearance),
                                    edge_clearance=MM(settings.m_CopperEdgeClearance),
                                    track_width=MM(settings.m_TrackMinWidth),
                                    via_size=MM(settings.m_ViasMinSize), through_drill=MM(settings.m_MinThroughDrill),
                                    annular_width=MM(settings.m_ViasMinAnnularWidth),
                                    hole_to_hole=MM(settings.m_HoleToHoleMin)))

    index = {}
    for pad in board.GetPads():
        net = pad.GetNetname() or None
        if net and pad.GetNetCode() > 0 and net not in picture.classes:
            cls = pad.GetNet().GetNetClassSlow()
            picture.classes[net] = NetClass(MM(cls.GetClearance()), MM(cls.GetTrackWidth()), MM(cls.GetViaDiameter()),
                                            MM(cls.GetViaDrill()))
        on = [l for l in enabled if pad.IsOnLayer(l)]
        shapes, openings = {}, {}
        for l in enabled:
            opening = mask_opening(pad, l)
            if l in on:
                shapes[name(l)] = rings_of(pad_polygon(pad, l))
            elif opening is not None:
                # an opening without copper under it (a pad on the other side's mask, or on no copper layer at all)
                shapes[name(l)] = rings_of(pad_outline(pad, on[0] if on else l))
            if opening is not None:
                openings[name(l)] = opening
        drill = pad.GetDrillSize()
        drilled = drill.x > 0
        index[pad.m_Uuid.AsString()] = len(picture.pads)
        picture.pads.append(Pad(net=net, at=xy(pad.GetPosition() if drilled or not on else pad_middle(pad, on[0])),
                                layers=[name(l) for l in on], shapes=shapes, openings=openings,
                                hole_at=xy(pad.GetPosition()) if drilled else None,
                                drill=MM(max(drill.x, drill.y)) if drilled else None,
                                clearance=local_clearance(pad), smallest=pad_smallest(pad, on),
                                part=pad.GetParentFootprint().GetReference() if pad.GetParentFootprint() else None))

    for footprint in board.GetFootprints():
        picture.parts[footprint.GetReference()] = Part(
            at=xy(footprint.GetPosition()), sheet=footprint.GetSheetname(), file=footprint.GetSheetfile(),
            value=footprint.GetValue(), footprint=footprint.GetFPIDAsString())

    for track in board.GetTracks():
        net = track.GetNetname() or None
        if track.Type() == pcbnew.PCB_VIA_T:
            try:
                width = MM(track.GetWidth(pcbnew.F_Cu))
            except TypeError:
                width = MM(track.GetWidth())
            try:
                drill = MM(track.GetDrillValue())
            except Exception:
                drill = MM(track.GetDrill())
            picture.vias.append(Via(at=xy(track.GetPosition()), width=width, drill=drill, net=net))
        elif track.Type() == pcbnew.PCB_ARC_T:
            # an arc as a few straight pieces
            poly = pcbnew.SHAPE_POLY_SET()
            track.TransformShapeToPolygon(poly, track.GetLayer(), 0, pcbnew.FromMM(ARC_ERROR), pcbnew.ERROR_OUTSIDE)
            picture.polygons.extend(Polygon(rings, name(track.GetLayer()), net) for rings in rings_of(poly))
        else:
            picture.tracks.append(Track(a=xy(track.GetStart()), b=xy(track.GetEnd()), width=MM(track.GetWidth()),
                                        net=net, layer=name(track.GetLayer())))

    # text and drawings on copper layers, on the board and in footprints: copper of no net
    drawn = list(board.GetDrawings())
    for footprint in board.GetFootprints():
        drawn.extend(footprint.GraphicalItems())
        drawn.extend(f for f in footprint.GetFields() if f.IsVisible())
    for item in drawn:
        for l in enabled:
            if item.IsOnLayer(l):
                picture.polygons.extend(Polygon(rings, name(l)) for rings in rings_of(drawing_polygon(item, l)))

    outline = pcbnew.SHAPE_POLY_SET()
    try:
        has_outline = board.GetBoardPolygonOutlines(outline, False)
    except TypeError:
        has_outline = board.GetBoardPolygonOutlines(outline)
    if has_outline:
        picture.outline = rings_of(outline)
    for zone in board.Zones():
        if not zone.GetIsRuleArea():
            continue  # filled copper is cut by new tracks when the zones are filled again
        if not (zone.GetDoNotAllowTracks() or zone.GetDoNotAllowVias()):
            continue
        on = [name(l) for l in enabled if zone.IsOnLayer(l)]
        for rings in rings_of(zone.Outline()):
            picture.areas.append(Area(rings=rings, layers=on, tracks=bool(zone.GetDoNotAllowTracks()),
                                      vias=bool(zone.GetDoNotAllowVias())))

    of = {}
    for net, own in pad_groups(board, copper=of).items():
        picture.groups[net] = [[index[p.m_Uuid.AsString()] for p in group if p.m_Uuid.AsString() in index]
                               for group in own]
    picture.fills = zone_fills(board) if pours else []
    if picture.fills:
        # what the zones join, and how they tie each pad: the engine's to work out from the plain geometry
        found = survey(picture.pads, zone_wires(board, of), picture.groups, picture.fills, spoked_pads(board, index),
                       ask)
        picture.groups = found["groups"]
        picture.pours = {net: {g: pieces for g, pieces in enumerate(own) if pieces}
                         for net, own in found["pours"].items()}
        picture.thermals = [Thermal(**thermal) for thermal in found["thermals"]]
    return picture


def write_back(board, solution, sizes, added=None):
    """Add a solution's tracks and vias to the board; `sizes` gives (track width, [via diameter, drill]) per
    entry key. Returns the numbers of track segments and vias added; `added`, if given, is filled with the
    items of each entry: {key: [item]}."""
    keep = lambda key, item: added.setdefault(key, []).append(item) if added is not None else None
    codes = {net.GetNetname(): net for net in board.GetNetsByName().values()}
    layer_id = {board.GetLayerName(l): l for l in copper_layers(board)}
    at = lambda p: pcbnew.VECTOR2I(pcbnew.FromMM(p[0]), pcbnew.FromMM(p[1]))
    tracks = vias = 0
    for laid in solution["nets"]:
        net, (width, via) = codes[laid["net"]], sizes[laid["key"]]
        for track in laid["tracks"]:
            for a, b in zip(track["points"], track["points"][1:]):
                item = pcbnew.PCB_TRACK(board)
                item.SetStart(at(a)); item.SetEnd(at(b)); item.SetWidth(pcbnew.FromMM(width))
                item.SetLayer(layer_id[track["layer"]]); item.SetNet(net)
                board.Add(item); tracks += 1; keep(laid["key"], item)
        for v in laid["vias"]:
            item = pcbnew.PCB_VIA(board)
            item.SetPosition(at(v)); item.SetViaType(pcbnew.VIATYPE_THROUGH)
            try:
                item.SetWidth(pcbnew.F_Cu, pcbnew.FromMM(via[0]))
            except TypeError:
                item.SetWidth(pcbnew.FromMM(via[0]))
            item.SetDrill(pcbnew.FromMM(via[1])); item.SetLayerPair(pcbnew.F_Cu, pcbnew.B_Cu); item.SetNet(net)
            board.Add(item); vias += 1; keep(laid["key"], item)
    return tracks, vias


class KicadBoard:
    """A pcbnew board as `route` uses it; it keeps the zones filled and the connectivity up to date with what
    has been written. `ask` puts a question to the engine; `pours`: what a filled zone joins counts as joined."""

    def __init__(self, board, ignored, ask, pours=True):
        self.board, self.ignored, self.ask, self.pours = board, ignored, ask, pours
        self.zoned = pours and any(not zone.GetIsRuleArea() for zone in board.Zones())
        self.stale = self.zoned  # (a board may come with its zones not filled)
        self.picture = None  # the board as it was last read, as long as nothing has been written since
        self.last = {}  # what the last solution written laid, by entry
        self.written = []  # and each one before it, in order
        self.out = []  # what was taken out of the board to be laid again, till it is known that it need not be put back

    def _fresh(self):
        if self.stale:
            if self.zoned:
                pcbnew.ZONE_FILLER(self.board).Fill(self.board.Zones())  # new tracks cut the pours
            self.board.BuildConnectivity()
            self.stale, self.picture = False, None

    def read(self, nets=None):
        self._fresh()
        if self.picture is None:
            self.picture = read_board(self.board, self.ignored, self.ask, self.pours)
        return self.picture

    def write(self, solution, sizes):
        self.stale, self.last = True, {}
        self.written.append(self.last)
        return write_back(self.board, solution, sizes, self.last)

    def mark(self):
        return len(self.written)

    def rewind(self, mark):
        tracks = vias = 0
        while len(self.written) > mark:
            self.last = self.written.pop()
            taken = self.take_back(list(self.last))
            tracks, vias = tracks + taken[0], vias + taken[1]
        self.last = {}
        return tracks, vias

    def take_back(self, keys):
        tracks = vias = 0
        for key in keys:
            for item in self.last.pop(key, []):
                via = item.Type() == pcbnew.PCB_VIA_T
                self.board.Remove(item)
                tracks, vias = tracks + (not via), vias + via
        self.stale = True
        return tracks, vias

    def take_out(self, nets):
        tracks = vias = 0
        self.out = []
        for record in self.written:
            for key, items in record.items():
                for item in [item for item in items if item.GetNetname() in nets]:
                    via = item.Type() == pcbnew.PCB_VIA_T
                    self.board.Remove(item)
                    items.remove(item)
                    self.out.append(item)
                    tracks, vias = tracks + (not via), vias + via
        self.stale = True
        return tracks, vias

    def put_back(self):
        tracks = vias = 0
        for item in self.out:
            via = item.Type() == pcbnew.PCB_VIA_T
            self.board.Add(item)
            tracks, vias = tracks + (not via), vias + via
        self.written.append({"put back": self.out})
        self.out, self.stale, self.last = [], True, {}
        return tracks, vias

    def unjoined(self, nets=None):
        if self.zoned:
            return self.read().unjoined(nets)
        self._fresh()
        return unjoined_nets(self.board, nets)
