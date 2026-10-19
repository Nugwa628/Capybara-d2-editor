#![allow(clippy::panic, clippy::unwrap_used)]

use super::*;
use proptest::prelude::*;

// ---------------------------------------------------------------------------
// An independent encoder written straight from the guide's "layout in one
// page" appendix. It shares no code with the reader/writer under test.

#[derive(Clone, Debug)]
struct Spec {
    version: i32,
    w: u32,
    h: u32,
    act: i32,
    tag_type: i32,
    files: Vec<Vec<u8>>,
    unknown: [i32; 2],
    walls: usize,
    floors: usize,
    objects: Vec<[i32; 5]>,
    pre_group: i32,
    groups: Vec<[i32; 5]>,
    paths: Vec<(i32, i32, Vec<[i32; 3]>)>,
    trailing: Vec<u8>,
    seed: u32,
}

impl Spec {
    fn new(version: i32) -> Spec {
        let walls = if version < 4 { 1 } else { 2 };
        Spec {
            version,
            w: 4,
            h: 3,
            act: 2,
            tag_type: 0,
            files: vec![
                b"\\d2\\data\\global\\tiles\\act1\\town\\floor.dt1".to_vec(),
                b"DS1EDIT_WRKSPC_TILE_X=3".to_vec(),
            ],
            unknown: [5000, 5000],
            walls,
            floors: if version < 16 { 1 } else { 2 },
            objects: vec![[2, 110, 164, 11, 0], [1, 5, 7, 8, 1]],
            pre_group: 0,
            groups: vec![[1, 2, 3, 4, 0], [5, 6, 7, 8, 9]],
            paths: vec![(7, 8, vec![[1, 2, 3], [4, 5, 6]])],
            trailing: Vec::new(),
            seed: 1,
        }
    }

    fn with_tag(mut self, tag_type: i32) -> Spec {
        self.tag_type = tag_type;
        self
    }

    fn has_tag(&self) -> bool {
        self.version < 4 || (self.version >= 10 && (1..=2).contains(&self.tag_type))
    }

    fn has_groups(&self) -> bool {
        self.version >= 12 && (1..=2).contains(&self.tag_type)
    }

    fn cells(&self, layer: u32) -> Vec<u32> {
        let n = (self.w * self.h) as usize;
        let mut s = self
            .seed
            .wrapping_mul(2654435761)
            .wrapping_add(layer * 977 + 13);
        (0..n)
            .map(|_| {
                s = s.wrapping_mul(1664525).wrapping_add(1013904223);
                // make some cells empty so both kinds exist
                if s >> 29 == 0 { 0 } else { s }
            })
            .collect()
    }

    fn encode(&self) -> Vec<u8> {
        let v = self.version;
        let mut b: Vec<u8> = Vec::new();
        let put = |b: &mut Vec<u8>, x: i32| b.extend_from_slice(&x.to_le_bytes());
        put(&mut b, v);
        put(&mut b, self.w as i32 - 1);
        put(&mut b, self.h as i32 - 1);
        if v >= 8 {
            put(&mut b, self.act);
        }
        if v >= 10 {
            put(&mut b, self.tag_type);
        }
        if v >= 3 {
            put(&mut b, self.files.len() as i32);
            for f in &self.files {
                b.extend_from_slice(f);
                b.push(0);
            }
        }
        if (9..=13).contains(&v) {
            put(&mut b, self.unknown[0]);
            put(&mut b, self.unknown[1]);
        }
        if v >= 4 {
            put(&mut b, self.walls as i32);
        }
        if v >= 16 {
            put(&mut b, self.floors as i32);
        }
        let dw = |b: &mut Vec<u8>, cells: Vec<u32>| {
            for c in cells {
                b.extend_from_slice(&c.to_le_bytes());
            }
        };
        if v >= 4 {
            for i in 0..self.walls as u32 {
                dw(&mut b, self.cells(10 + i));
                dw(&mut b, self.cells(20 + i));
            }
            for i in 0..self.floors as u32 {
                dw(&mut b, self.cells(30 + i));
            }
            dw(&mut b, self.cells(40));
            if self.has_tag() {
                dw(&mut b, self.cells(50));
            }
        } else {
            dw(&mut b, self.cells(10));
            dw(&mut b, self.cells(30));
            dw(&mut b, self.cells(20));
            dw(&mut b, self.cells(50));
            dw(&mut b, self.cells(40));
        }
        if v >= 2 {
            put(&mut b, self.objects.len() as i32);
            for o in &self.objects {
                for &x in &o[..4] {
                    put(&mut b, x);
                }
                if v >= 6 {
                    put(&mut b, o[4]);
                }
            }
        }
        if self.has_groups() {
            if v >= 18 {
                put(&mut b, self.pre_group);
            }
            put(&mut b, self.groups.len() as i32);
            for g in &self.groups {
                for &x in &g[..4] {
                    put(&mut b, x);
                }
                if v >= 13 {
                    put(&mut b, g[4]);
                }
            }
        }
        if v >= 14 {
            put(&mut b, self.paths.len() as i32);
            for (x, y, pts) in &self.paths {
                put(&mut b, pts.len() as i32);
                put(&mut b, *x);
                put(&mut b, *y);
                for p in pts {
                    put(&mut b, p[0]);
                    put(&mut b, p[1]);
                    if v >= 15 {
                        put(&mut b, p[2]);
                    }
                }
            }
        }
        b.extend_from_slice(&self.trailing);
        b
    }

    /// The model `parse` should produce for `encode()`'s output.
    fn expected(&self) -> Ds1 {
        let v = self.version;
        let g = |layer: u32| Grid::from_vec(self.w, self.h, self.cells(layer)).unwrap();
        let cg = |layer: u32| {
            Grid::from_vec(
                self.w,
                self.h,
                self.cells(layer).into_iter().map(Cell).collect(),
            )
            .unwrap()
        };
        let walls = if v >= 4 {
            (0..self.walls as u32)
                .map(|i| WallLayer {
                    tiles: cg(10 + i),
                    orientations: g(20 + i),
                })
                .collect()
        } else {
            vec![WallLayer {
                tiles: cg(10),
                orientations: g(20),
            }]
        };
        let floors = if v >= 4 {
            (0..self.floors as u32).map(|i| cg(30 + i)).collect()
        } else {
            vec![cg(30)]
        };
        let mut preserved = Preserved::default();
        if (9..=13).contains(&v) {
            preserved.unknown_v9_13 = self.unknown;
        }
        if self.has_groups() && v >= 18 {
            preserved.pre_group_dword = self.pre_group;
        }
        preserved.trailing = self.trailing.clone();
        Ds1 {
            version: v,
            width: self.w,
            height: self.h,
            act: if v >= 8 { self.act } else { 0 },
            tag_type: if v >= 10 { self.tag_type } else { 0 },
            files: if v >= 3 { self.files.clone() } else { vec![] },
            walls,
            floors,
            shadow: cg(40),
            tag: self.has_tag().then(|| g(50)),
            objects: if v >= 2 {
                self.objects
                    .iter()
                    .map(|o| Object {
                        kind: o[0],
                        id: o[1],
                        x: o[2],
                        y: o[3],
                        flags: if v >= 6 { o[4] } else { 0 },
                    })
                    .collect()
            } else {
                vec![]
            },
            groups: if self.has_groups() {
                self.groups
                    .iter()
                    .map(|g| Group {
                        x: g[0],
                        y: g[1],
                        width: g[2],
                        height: g[3],
                        unknown: if v >= 13 { g[4] } else { 0 },
                    })
                    .collect()
            } else {
                vec![]
            },
            npc_paths: if v >= 14 {
                self.paths
                    .iter()
                    .map(|(x, y, pts)| NpcPath {
                        x: *x,
                        y: *y,
                        points: pts
                            .iter()
                            .map(|p| PathPoint {
                                x: p[0],
                                y: p[1],
                                action: if v >= 15 { p[2] } else { 1 },
                            })
                            .collect(),
                    })
                    .collect()
            } else {
                vec![]
            },
            preserved,
        }
    }
}

fn check_round_trip(spec: &Spec) {
    let bytes = spec.encode();
    let (ds1, report) = Ds1::parse_with_report(&bytes)
        .unwrap_or_else(|e| panic!("v{} tag{}: {e}", spec.version, spec.tag_type));
    assert_eq!(
        ds1,
        spec.expected(),
        "v{} tag{}",
        spec.version,
        spec.tag_type
    );
    assert_eq!(report.trailing_bytes, spec.trailing.len());
    assert_eq!(report.missing_bytes, 0);
    assert_eq!(
        ds1.to_bytes().unwrap(),
        bytes,
        "v{} tag{} write",
        spec.version,
        spec.tag_type
    );
}

const VERSIONS: [i32; 21] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 40, -7,
];

#[test]
fn every_gate_version_round_trips() {
    for &v in &VERSIONS {
        for tag_type in 0..=2 {
            check_round_trip(&Spec::new(v).with_tag(tag_type));
        }
    }
}

#[test]
fn odd_tag_types_have_no_tag_or_groups() {
    for tag_type in [-1, 3, 99] {
        let spec = Spec::new(18).with_tag(tag_type);
        let ds1 = Ds1::parse(&spec.encode()).unwrap();
        assert!(ds1.tag.is_none());
        assert!(ds1.groups.is_empty());
        check_round_trip(&spec);
    }
}

#[test]
fn layer_count_extremes() {
    for &v in &[4, 8, 15, 16, 18] {
        for walls in [0usize, 1, 4] {
            for floors in [0usize, 1, 2] {
                if v < 16 && floors != 1 {
                    continue;
                }
                let mut spec = Spec::new(v).with_tag(1);
                spec.walls = walls;
                spec.floors = floors;
                check_round_trip(&spec);
                let ds1 = Ds1::parse(&spec.encode()).unwrap();
                assert_eq!(ds1.walls.len(), walls);
                assert_eq!(ds1.floors.len(), floors);
            }
        }
    }
}

#[test]
fn empty_collections_and_zero_size() {
    for &v in &[2, 14, 18] {
        let mut spec = Spec::new(v).with_tag(1);
        spec.objects.clear();
        spec.groups.clear();
        spec.paths.clear();
        spec.files.clear();
        check_round_trip(&spec);
    }
    // width/height of zero are legal (stored value -1) and have empty layers
    let mut spec = Spec::new(18);
    spec.w = 0;
    spec.h = 5;
    check_round_trip(&spec);
}

#[test]
fn trailing_bytes_are_preserved() {
    for &v in &VERSIONS {
        for trailing in [vec![0u8; 4], vec![1, 2, 3], (0..48).collect::<Vec<u8>>()] {
            let mut spec = Spec::new(v).with_tag(1);
            spec.trailing = trailing;
            check_round_trip(&spec);
        }
    }
}

#[test]
fn trailing_group_records_like_swamp2() {
    // v13, one wall absent, declared 2 groups but 2 more well-formed ones follow.
    let mut spec = Spec::new(13).with_tag(1);
    spec.walls = 0;
    spec.groups = vec![[0, 0, 1, 1, 0], [2, 2, 1, 1, 7]];
    let mut bytes = spec.encode();
    let extra: Vec<i32> = vec![1, 5, 3, 3, 0, 15, 0, 0, 0, 0, 0, 0];
    for x in &extra {
        bytes.extend_from_slice(&x.to_le_bytes());
    }
    let (ds1, report) = Ds1::parse_with_report(&bytes).unwrap();
    assert_eq!(report.trailing_bytes, 48);
    assert_eq!(ds1.preserved.trailing.len(), 48);
    assert_eq!(ds1.to_bytes().unwrap(), bytes);
}

#[test]
fn missing_bytes_read_as_zero_and_round_trip() {
    // No objects/groups/paths so the structure ends in zeros.
    for &v in &[3, 8, 13, 14, 15, 18] {
        let mut spec = Spec::new(v).with_tag(1);
        spec.objects.clear();
        spec.groups.clear();
        spec.paths.clear();
        let full = spec.encode();
        // object count, [pre-group dword], group count, npc count
        let tail =
            4 + if spec.has_groups() {
                4 + if v >= 18 { 4 } else { 0 }
            } else {
                0
            } + if v >= 14 { 4 } else { 0 };
        for cut in 1..=tail {
            let bytes = &full[..full.len() - cut];
            let (ds1, report) = Ds1::parse_with_report(bytes).unwrap();
            assert_eq!(report.missing_bytes, cut, "v{v} cut{cut}");
            assert_eq!(ds1.preserved.missing, cut);
            assert!(ds1.preserved.trailing.is_empty());
            // the zero-filled model equals the untruncated one except `missing`
            let mut expected = spec.expected();
            expected.preserved.missing = cut;
            assert_eq!(ds1, expected, "v{v} cut{cut}");
            assert_eq!(ds1.to_bytes().unwrap(), bytes, "v{v} cut{cut}");
        }
    }
}

#[test]
fn missing_bytes_midway_through_object_block_round_trip() {
    let mut spec = Spec::new(18);
    spec.objects = (0..40).map(|i| [1, i + 1, i, i * 2, 3]).collect();
    // no paths: the file is layers, object block, then a lone npc count
    spec.paths.clear();
    let full = spec.encode();
    let npc_count = 4;
    for cut in [1usize, 7, 12, 100, MAX_MISSING_BYTES - npc_count] {
        let bytes = &full[..full.len() - npc_count - cut];
        let (ds1, report) = Ds1::parse_with_report(bytes).unwrap();
        assert_eq!(report.missing_bytes, cut + npc_count, "cut {cut}");
        assert_eq!(ds1.objects.len(), 40);
        assert_eq!(ds1.to_bytes().unwrap(), bytes);
    }
    // one byte past the allowed slack is an error
    let bytes = &full[..full.len() - (MAX_MISSING_BYTES + 1)];
    assert!(matches!(
        Ds1::parse(bytes),
        Err(FormatError::Truncated { .. })
    ));
}

#[test]
fn cell_layers_must_be_complete() {
    let full = Spec::new(18).encode();
    // cutting into the layers (the object block here is tiny) is an error
    let layers_end = full.len() - (4 + 2 * 20) - (4 + 12 + 24);
    for cut in [layers_end - 1, layers_end - 4, 12, 20] {
        assert!(
            matches!(Ds1::parse(&full[..cut]), Err(FormatError::Truncated { .. })),
            "cut at {cut}"
        );
    }
}

#[test]
fn edited_missing_region_is_kept() {
    let mut spec = Spec::new(18);
    spec.objects.clear();
    spec.paths.clear();
    let full = spec.encode();
    let mut ds1 = Ds1::parse(&full[..full.len() - 8]).unwrap();
    assert_eq!(ds1.preserved.missing, 8);
    ds1.objects.push(Object {
        kind: 1,
        id: 2,
        x: 3,
        y: 4,
        flags: 0,
    });
    let out = ds1.to_bytes().unwrap();
    assert!(out.len() > full.len() - 8);
    assert_eq!(Ds1::parse(&out).unwrap().objects.len(), 1);
}

#[test]
fn hostile_headers_are_rejected_without_huge_allocations() {
    let mut b = Vec::new();
    for x in [18i32, 0x7FFF_FFFE, 0x7FFF_FFFE, 0, 0, 0, 1, 1] {
        b.extend_from_slice(&x.to_le_bytes());
    }
    assert!(Ds1::parse(&b).is_err());

    // negative dimension
    let mut b = Vec::new();
    for x in [18i32, -5, 3] {
        b.extend_from_slice(&x.to_le_bytes());
    }
    assert!(matches!(Ds1::parse(&b), Err(FormatError::Invalid { .. })));

    // five walls
    let mut spec = Spec::new(18);
    spec.walls = 5;
    assert!(matches!(
        Ds1::parse(&spec.encode()),
        Err(FormatError::Invalid { .. })
    ));
    // three floors
    let mut spec = Spec::new(18);
    spec.floors = 3;
    assert!(Ds1::parse(&spec.encode()).is_err());

    // enormous file count / object count / group count / path counts
    for prefix in [vec![18i32, 1, 1, 0, 0], vec![18, 1, 1, 0, 1]] {
        // enormous file count, then enormous layer count
        let mut b = Vec::new();
        for x in prefix {
            b.extend_from_slice(&x.to_le_bytes());
        }
        b.extend_from_slice(&i32::MAX.to_le_bytes());
        assert!(Ds1::parse(&b).is_err());
    }
    // enormous object / group / path counts after valid layers
    let mut hostile = Spec::new(18).with_tag(1);
    hostile.objects.clear();
    hostile.groups.clear();
    hostile.paths.clear();
    let clean = hostile.encode();
    // tail: object count, pre-group dword, group count, npc count
    let obj_count_at = clean.len() - 16;
    for at in [obj_count_at, obj_count_at + 8, obj_count_at + 12] {
        let mut b = clean.clone();
        b[at..at + 4].copy_from_slice(&i32::MAX.to_le_bytes());
        assert!(Ds1::parse(&b).is_err(), "huge count at {at}");
        b[at..at + 4].copy_from_slice(&(-1i32).to_le_bytes());
        assert!(Ds1::parse(&b).is_err(), "negative count at {at}");
    }
    assert!(Ds1::parse(&[]).is_err());
    assert!(Ds1::parse(&[1, 2, 3]).is_err());
}

#[test]
fn not_representable_cases() {
    // two walls at v3
    let mut ds1 = Ds1::parse(&Spec::new(3).encode()).unwrap();
    ds1.walls.push(ds1.walls[0].clone());
    assert!(matches!(
        ds1.to_bytes(),
        Err(FormatError::NotRepresentable { .. })
    ));
    // two floors at v15
    let mut ds1 = Ds1::parse(&Spec::new(15).encode()).unwrap();
    ds1.floors.push(ds1.floors[0].clone());
    assert!(matches!(
        ds1.to_bytes(),
        Err(FormatError::NotRepresentable { .. })
    ));
    // objects at v1
    let mut ds1 = Ds1::new(2, 2, 0);
    ds1.version = 1;
    ds1.files.clear();
    ds1.tag_type = 0;
    ds1.tag = Some(Grid::new(2, 2));
    assert!(ds1.to_bytes().is_ok());
    ds1.objects.push(Object {
        kind: 1,
        id: 1,
        x: 0,
        y: 0,
        flags: 0,
    });
    assert!(ds1.to_bytes().is_err());
    // inconsistent tag layer at v18
    let mut ds1 = Ds1::new(2, 2, 0);
    ds1.tag = Some(Grid::new(2, 2));
    assert!(ds1.to_bytes().is_err());
    ds1.tag = None;
    ds1.tag_type = 1;
    assert!(ds1.to_bytes().is_err());
    ds1.tag = Some(Grid::new(2, 2));
    assert!(ds1.to_bytes().is_ok());
    // a layer of the wrong size
    let mut ds1 = Ds1::new(2, 2, 0);
    ds1.shadow = Grid::new(3, 2);
    assert!(ds1.to_bytes().is_err());
    // a NUL inside a file string
    let mut ds1 = Ds1::new(2, 2, 0);
    ds1.files.push(b"a\0b".to_vec());
    assert!(ds1.to_bytes().is_err());
}

#[test]
fn add_layers_respect_limits() {
    let mut ds1 = Ds1::new(3, 2, 0);
    assert_eq!(ds1.add_wall_layer().unwrap(), 1);
    assert_eq!(ds1.add_wall_layer().unwrap(), 2);
    assert_eq!(ds1.add_wall_layer().unwrap(), 3);
    assert!(ds1.add_wall_layer().is_err());
    assert_eq!(ds1.walls.len(), MAX_WALL_LAYERS);
    assert_eq!(ds1.add_floor_layer().unwrap(), 1);
    assert!(ds1.add_floor_layer().is_err());
    assert_eq!(ds1.floors.len(), MAX_FLOOR_LAYERS);
    assert_eq!(ds1.walls[3].tiles.width(), 3);
    let reparsed = Ds1::parse(&ds1.to_bytes().unwrap()).unwrap();
    assert_eq!(reparsed, ds1);

    let mut old = Ds1::parse(&Spec::new(3).encode()).unwrap();
    assert!(old.add_wall_layer().is_err());
    assert!(old.add_floor_layer().is_err());
    let mut mid = Ds1::parse(&Spec::new(15).encode()).unwrap();
    assert!(mid.add_wall_layer().is_ok());
    assert!(mid.add_floor_layer().is_err());
}

#[test]
fn min_version_for_model_values() {
    // the search starts at the model's own version
    let ds1 = Ds1::new(2, 2, 0);
    assert_eq!(ds1.min_version_for_model(), 18);

    // A model parsed at v3 with a second wall added is not valid at 3, needs 4.
    let mut ds1 = Ds1::parse(&Spec::new(3).encode()).unwrap();
    let extra = ds1.walls[0].clone();
    ds1.walls.push(extra);
    // v4+ drops the tag layer rule: tag must be absent unless tag_type 1/2
    ds1.tag = None;
    assert_eq!(ds1.min_version_for_model(), 4);
    ds1.floors.push(ds1.floors[0].clone());
    assert_eq!(ds1.min_version_for_model(), 16);

    // groups need v12; paths need v14; actions != 1 need v15; unknown group dword v13
    let mut ds1 = Ds1::parse(&Spec::new(10).with_tag(1).encode()).unwrap();
    assert!(ds1.groups.is_empty());
    assert_eq!(ds1.min_version_for_model(), 10);
    ds1.groups.push(Group {
        x: 1,
        y: 1,
        width: 1,
        height: 1,
        unknown: 0,
    });
    assert_eq!(ds1.min_version_for_model(), 12);
    ds1.groups[0].unknown = 5;
    // the v9..=13 unknown pair is still held at its stored value, which only
    // fits while the version stays in 9..=13
    assert_eq!(ds1.min_version_for_model(), 13);
    ds1.preserved.unknown_v9_13 = [0, 0];
    ds1.npc_paths.push(NpcPath {
        x: 0,
        y: 0,
        points: vec![PathPoint {
            x: 0,
            y: 0,
            action: 1,
        }],
    });
    assert_eq!(ds1.min_version_for_model(), 14);
    ds1.npc_paths[0].points[0].action = 3;
    assert_eq!(ds1.min_version_for_model(), 15);
    ds1.act = 2;
    ds1.objects.push(Object {
        kind: 1,
        id: 1,
        x: 1,
        y: 1,
        flags: 1,
    });
    assert_eq!(ds1.min_version_for_model(), 15);

    // object flags need v6, act needs v8
    let mut ds1 = Ds1::parse(&Spec::new(4).encode()).unwrap();
    assert_eq!(ds1.min_version_for_model(), 4);
    ds1.objects[0].flags = 1;
    assert_eq!(ds1.min_version_for_model(), 6);
    ds1.act = 3;
    assert_eq!(ds1.min_version_for_model(), 8);
    ds1.tag_type = 2;
    ds1.tag = Some(Grid::new(ds1.width, ds1.height));
    assert_eq!(ds1.min_version_for_model(), 10);

    // an impossible model falls back to max(version, 18)
    let mut ds1 = Ds1::new(2, 2, 0);
    ds1.tag = Some(Grid::new(2, 2));
    assert_eq!(ds1.min_version_for_model(), 18);
}

#[test]
fn cell_accessors() {
    let c = Cell(0x8000_0000 | (30 << 20) | (7 << 8) | 0x81 | (1 << 17));
    assert_eq!(c.prop1(), 0x81);
    assert_eq!(c.sub_index(), 7);
    assert_eq!(c.main_index(), 30);
    assert!(c.unwalkable());
    assert!(c.hidden());
    assert!(!c.is_empty());
    assert!(Cell::EMPTY.is_empty());
    // the guide's worked example: floor cell 0x005000C2
    let f = Cell(0x0050_00C2);
    assert_eq!((f.prop1(), f.sub_index(), f.main_index()), (0xC2, 0, 5));
    // with_indices keeps every other bit, including bit 26 (not part of main)
    let g = Cell(0x0400_0000 | 0x8002_0081).with_indices(33, 9);
    assert_eq!(g.main_index(), 33);
    assert_eq!(g.sub_index(), 9);
    assert!(g.hidden() && g.unwalkable());
    assert_eq!(g.prop1(), 0x81);
    assert_ne!(g.0 & 0x0400_0000, 0);
    assert_eq!(g.with_hidden(false).0 & 0x8000_0000, 0);
    assert!(g.with_hidden(false).with_hidden(true).hidden());
    // main is masked to six bits
    assert_eq!(Cell::EMPTY.with_indices(0xFF, 0).main_index(), 0x3F);
    assert_eq!(std::mem::size_of::<Cell>(), 4);
}

#[test]
fn orientation_lookup_applies_below_v7() {
    let mut old = Ds1::parse(&Spec::new(6).encode()).unwrap();
    let mut new = Ds1::parse(&Spec::new(7).encode()).unwrap();
    for ds1 in [&mut old, &mut new] {
        ds1.walls[0].orientations.set(0, 0, 7);
        ds1.walls[0].orientations.set(1, 0, 0x1234_5603);
        ds1.walls[0].orientations.set(2, 0, 24);
        ds1.walls[0].orientations.set(3, 0, 25);
    }
    assert_eq!(old.orientation(0, 0, 0), Some(DIR_LOOKUP[7]));
    assert_eq!(old.orientation(0, 0, 0), Some(5));
    assert_eq!(old.orientation(0, 1, 0), Some(DIR_LOOKUP[3]));
    assert_eq!(old.orientation(0, 2, 0), Some(20));
    assert_eq!(old.orientation(0, 3, 0), None);
    assert_eq!(new.orientation(0, 0, 0), Some(7));
    assert_eq!(new.orientation(0, 1, 0), Some(3));
    assert_eq!(new.orientation(0, 3, 0), Some(25));
    assert_eq!(new.orientation(5, 0, 0), None);
    assert_eq!(new.orientation(0, 99, 0), None);
    // raw dwords are never altered
    assert_eq!(old.walls[0].orientations.get(1, 0), Some(0x1234_5603));
}

#[test]
fn dir_lookup_matches_the_guide() {
    assert_eq!(
        DIR_LOOKUP,
        [
            0, 1, 2, 1, 2, 3, 3, 5, 5, 6, 6, 7, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 20
        ]
    );
}

#[test]
fn act_and_file_helpers() {
    let mut ds1 = Ds1::new(2, 2, 5);
    assert_eq!(ds1.effective_act(), 4);
    ds1.act = -3;
    assert_eq!(ds1.effective_act(), 0);
    ds1.act = 2;
    assert_eq!(ds1.effective_act(), 2);

    ds1.files = vec![
        b"\\d2\\data\\global\\tiles\\a.dt1".to_vec(),
        b"DS1EDIT_WRKSPC_TILE_X=1".to_vec(),
        b"\\d2\\b.dt1".to_vec(),
        b"DS1EDIT_WRKSPC_ZOOM=2".to_vec(),
    ];
    assert_eq!(ds1.workspace_entries().count(), 2);
    let refs: Vec<&[u8]> = ds1.tileset_refs().collect();
    assert_eq!(
        refs,
        [&b"\\d2\\data\\global\\tiles\\a.dt1"[..], b"\\d2\\b.dt1"]
    );
    assert!(Ds1::parse(&ds1.to_bytes().unwrap()).unwrap() == ds1);
}

#[test]
fn new_is_v18_blank() {
    let ds1 = Ds1::new(5, 4, 1);
    assert_eq!(ds1.version, 18);
    assert_eq!((ds1.walls.len(), ds1.floors.len()), (1, 1));
    assert!(ds1.tag.is_none());
    assert_eq!(ds1.shadow.as_slice().len(), 20);
    let bytes = ds1.to_bytes().unwrap();
    assert_eq!(Ds1::parse(&bytes).unwrap(), ds1);
}

#[test]
fn report_collects_warnings() {
    let mut spec = Spec::new(18).with_tag(1);
    spec.act = 5;
    spec.objects = vec![[1, 1, 9999, 0, 0], [1, 1, 3, 3, 0], [1, 1, 3, 3, 0]];
    spec.paths = vec![(3, 3, vec![[0, 0, 1]]), (50, 50, vec![])];
    let (_, report) = Ds1::parse_with_report(&spec.encode()).unwrap();
    let all = report.warnings.join("\n");
    assert!(all.contains("act 5"), "{all}");
    assert!(all.contains("outside the sub-tile grid"), "{all}");
    assert!(all.contains("matches 2 objects"), "{all}");
    assert!(all.contains("matches 0 objects"), "{all}");
}

// ---------------------------------------------------------------------------
// property tests

fn arb_cells(n: usize) -> impl Strategy<Value = Vec<u32>> {
    prop::collection::vec(any::<u32>(), n)
}

fn arb_model() -> impl Strategy<Value = Ds1> {
    (
        0u32..6,
        0u32..6,
        any::<i32>(),
        -1i32..4,
        prop::collection::vec(prop::collection::vec(1u8..=255, 0..20), 0..4),
        0usize..=4,
        0usize..=2,
    )
        .prop_flat_map(|(w, h, act, tag_type, files, walls, floors)| {
            let n = (w * h) as usize;
            let tagged = (1..=2).contains(&tag_type);
            (
                Just((w, h, act, tag_type, files)),
                prop::collection::vec((arb_cells(n), arb_cells(n)), walls),
                prop::collection::vec(arb_cells(n), floors),
                arb_cells(n),
                arb_cells(if tagged { n } else { 0 }),
                prop::collection::vec(any::<[i32; 5]>(), 0..6),
                prop::collection::vec(any::<[i32; 5]>(), 0..4),
                prop::collection::vec(
                    (
                        any::<i32>(),
                        any::<i32>(),
                        prop::collection::vec(any::<[i32; 3]>(), 0..4),
                    ),
                    0..4,
                ),
                any::<i32>(),
                prop::collection::vec(any::<u8>(), 0..24),
            )
        })
        .prop_map(
            |(
                (w, h, act, tag_type, files),
                walls,
                floors,
                shadow,
                tag,
                objects,
                groups,
                paths,
                pre_group,
                trailing,
            )| {
                let g = |c: Vec<u32>| Grid::from_vec(w, h, c).unwrap();
                let cg = |c: Vec<u32>| g(c.clone());
                let _ = cg;
                let tagged = (1..=2).contains(&tag_type);
                Ds1 {
                    version: 18,
                    width: w,
                    height: h,
                    act,
                    tag_type,
                    files,
                    walls: walls
                        .into_iter()
                        .map(|(t, o)| WallLayer {
                            tiles: Grid::from_vec(w, h, t.into_iter().map(Cell).collect()).unwrap(),
                            orientations: g(o),
                        })
                        .collect(),
                    floors: floors
                        .into_iter()
                        .map(|f| Grid::from_vec(w, h, f.into_iter().map(Cell).collect()).unwrap())
                        .collect(),
                    shadow: Grid::from_vec(w, h, shadow.into_iter().map(Cell).collect()).unwrap(),
                    tag: tagged.then(|| g(tag)),
                    objects: objects
                        .into_iter()
                        .map(|o| Object {
                            kind: o[0],
                            id: o[1],
                            x: o[2],
                            y: o[3],
                            flags: o[4],
                        })
                        .collect(),
                    groups: if tagged {
                        groups
                            .into_iter()
                            .map(|o| Group {
                                x: o[0],
                                y: o[1],
                                width: o[2],
                                height: o[3],
                                unknown: o[4],
                            })
                            .collect()
                    } else {
                        vec![]
                    },
                    npc_paths: paths
                        .into_iter()
                        .map(|(x, y, pts)| NpcPath {
                            x,
                            y,
                            points: pts
                                .into_iter()
                                .map(|p| PathPoint {
                                    x: p[0],
                                    y: p[1],
                                    action: p[2],
                                })
                                .collect(),
                        })
                        .collect(),
                    preserved: Preserved {
                        unknown_v9_13: [0, 0],
                        pre_group_dword: if tagged { pre_group } else { 0 },
                        trailing,
                        missing: 0,
                    },
                }
            },
        )
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    #[test]
    fn random_v18_models_round_trip(model in arb_model()) {
        let bytes = model.to_bytes().unwrap();
        let parsed = Ds1::parse(&bytes).unwrap();
        prop_assert_eq!(&parsed, &model);
        prop_assert_eq!(parsed.to_bytes().unwrap(), bytes);
    }

    #[test]
    fn random_bytes_never_panic(bytes in prop::collection::vec(any::<u8>(), 0..300)) {
        let _ = Ds1::parse_with_report(&bytes);
    }

    #[test]
    fn mutated_files_never_panic_and_round_trip_when_they_parse(
        version in prop::sample::select(VERSIONS.to_vec()),
        tag_type in 0i32..3,
        flips in prop::collection::vec((any::<prop::sample::Index>(), any::<u8>()), 0..6),
        cut in 0usize..40,
        append in prop::collection::vec(any::<u8>(), 0..8),
    ) {
        let mut bytes = Spec::new(version).with_tag(tag_type).encode();
        for (idx, val) in flips {
            let i = idx.index(bytes.len());
            bytes[i] = val;
        }
        let keep = bytes.len().saturating_sub(cut);
        bytes.truncate(keep);
        bytes.extend_from_slice(&append);
        if let Ok(ds1) = Ds1::parse(&bytes) {
            prop_assert_eq!(ds1.to_bytes().unwrap(), bytes);
        }
    }
}

// ---------------------------------------------------------------------------
// real-data corpus

#[test]
fn corpus_round_trips_byte_identically() {
    let mut found_any = false;
    for name in ["vanilla", "mxl"] {
        let Some(dir) = crate::testutil::extracted(name) else {
            println!("skipped: extracted/{name} missing");
            continue;
        };
        found_any = true;
        let files = crate::testutil::find_files(&dir, "ds1");
        if files.is_empty() {
            println!("skipped: no .ds1 under {}", dir.display());
            continue;
        }
        let mut by_version: std::collections::BTreeMap<i32, usize> = Default::default();
        let mut with_trailing = Vec::new();
        let mut with_missing = Vec::new();
        let mut warnings = 0usize;
        let mut failures = Vec::new();
        for path in &files {
            let bytes = std::fs::read(path).unwrap();
            let shown = path.strip_prefix(&dir).unwrap().display().to_string();
            match Ds1::parse_with_report(&bytes) {
                Err(e) => failures.push(format!("{shown}: parse: {e}")),
                Ok((ds1, report)) => {
                    *by_version.entry(ds1.version).or_default() += 1;
                    if report.trailing_bytes > 0 {
                        with_trailing.push(format!("{shown} (+{})", report.trailing_bytes));
                    }
                    if report.missing_bytes > 0 {
                        with_missing.push(format!("{shown} (-{})", report.missing_bytes));
                    }
                    warnings += report.warnings.len();
                    match ds1.to_bytes() {
                        Err(e) => failures.push(format!("{shown}: write: {e}")),
                        Ok(out) if out != bytes => {
                            failures.push(format!("{shown}: written bytes differ"));
                        }
                        Ok(_) => {}
                    }
                }
            }
        }
        println!("== {name}: {} .ds1 files", files.len());
        println!("   versions: {by_version:?}");
        println!(
            "   trailing bytes ({}): {with_trailing:?}",
            with_trailing.len()
        );
        println!(
            "   missing bytes ({}): {with_missing:?}",
            with_missing.len()
        );
        println!("   warnings: {warnings}");
        println!("   exceptions ({}): {failures:#?}", failures.len());
        assert!(failures.is_empty(), "{name}: {failures:#?}");
    }
    if !found_any {
        println!("skipped: extracted corpus missing");
    }
}
