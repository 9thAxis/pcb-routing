"""The KiCad bridge of pcb-routing. Its command line is tools/kicad_route.py.

A board is read into a picture (plain data), the picture and the board's rules become the router's problem, the
engine runs it, and what it laid is written back; then the zones are filled again and what the new tracks did to
them is mended. Only `kicad` touches KiCad.

    picture    a board as plain data: pads, tracks, vias, zones as filled, groups of joined pads
    names      what the names of nets and parts say they are (supplies, pairs, clocks, crystals)
    intent     what the nets are for: guesses from names, and the hints file beside the board
    rules      the board's rules as the router's distances and sizes, and the tries a lot of nets gets
    questions  what is asked of the engine about zones (the geometry is the crate's: src/pours)
    problem    a picture and the rules as the router's problem: entries, pairs, links of capacitors, buses
    engine     the pcb-routing program: runs and questions
    route      joining the nets: lots, tries, what is done over and why
    ties       pads left with too few spokes to their zone
    kicad      reading a pcbnew board into a picture and writing tracks and vias back
"""
