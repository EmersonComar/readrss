//! O ícone do app (usado pelo .desktop, pelo Sobre e futuramente pelo snap).

use std::collections::BTreeSet;
use std::path::PathBuf;

use gtk::gdk_pixbuf::Pixbuf;
use quick_xml::Reader;
use quick_xml::events::Event;
use readrss::constants::{APP_ID, FILTER_ICON};

fn icon(kind: &str, name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("data/icons/hicolor/{kind}/apps/{name}.svg"))
}

/// (viewBox da raiz, atributos de todos os elementos)
fn svg_attributes(path: &PathBuf) -> (String, Vec<Vec<(String, String)>>) {
    let text = std::fs::read_to_string(path).unwrap();
    let mut reader = Reader::from_str(&text);
    let (mut view_box, mut elements) = (String::new(), Vec::new());
    loop {
        match reader.read_event().unwrap() {
            Event::Start(e) | Event::Empty(e) => {
                let attrs: Vec<(String, String)> = e
                    .attributes()
                    .flatten()
                    .map(|a| (a.key.as_ref().to_string(), a.value.to_string()))
                    .collect();
                if e.name().as_ref() == "svg" {
                    view_box = attrs.iter().find(|(k, _)| k == "viewBox").map(|(_, v)| v.clone()).unwrap_or_default();
                }
                elements.push(attrs);
            }
            Event::Eof => break,
            _ => {}
        }
    }
    (view_box, elements)
}

#[test]
fn icons_render_at_every_size() {
    let symbolic = format!("{APP_ID}-symbolic");
    for path in [icon("scalable", APP_ID), icon("symbolic", &symbolic), icon("symbolic", FILTER_ICON)] {
        for size in [16, 32, 64, 128] {
            let pixbuf = Pixbuf::from_file_at_size(&path, size, size).unwrap();
            assert_eq!((pixbuf.width(), pixbuf.height()), (size, size), "{path:?}");
            let pixels = pixbuf.read_pixel_bytes();
            assert!(pixels.chunks(4).any(|p| p[3] > 0), "{path:?} renderizou vazio em {size}px");
        }
    }
}

#[test]
fn app_icon_uses_gnome_128_grid() {
    assert_eq!(svg_attributes(&icon("scalable", APP_ID)).0, "0 0 128 128");
}

#[test]
fn symbolic_icons_are_recolorable() {
    for path in [icon("symbolic", &format!("{APP_ID}-symbolic")), icon("symbolic", FILTER_ICON)] {
        let (view_box, elements) = svg_attributes(&path);
        assert_eq!(view_box, "0 0 16 16", "{path:?}");
        let mut fills = BTreeSet::new();
        for attrs in &elements {
            for (key, value) in attrs {
                assert!(!key.starts_with("stroke") && !(key == "style" && value.contains("stroke")),
                        "{path:?}: o GTK só recolore preenchimentos ({key}={value})");
                if key == "fill" {
                    fills.insert(value.clone());
                }
            }
        }
        assert_eq!(fills.len(), 1, "{path:?}: uma única cor ({fills:?})");
    }
}
