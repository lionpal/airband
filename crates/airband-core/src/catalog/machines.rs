//! What shape each computer is, by the identifier its system gives its model (`catalog/machines/`). A row is
//! one line of machines, the shape they share, and every model identifier they go by, matched exactly as the
//! system gives it: nothing is read into an identifier's letters or numbers, and one no row lists has no shape.
use super::*;

/// The shape of a computer, as the rows give it.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Shape {
    Laptop,
    Mini,
    Studio,
    Imac,
    Pro,
}

/// One line of machines: the shape they share and the identifiers they go by.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Machines {
    pub id: String,
    pub shape: Shape,
    pub models: Vec<String>,
}

impl Catalog {
    /// The shape of the computer whose model goes by this identifier. Nothing when no row lists it exactly.
    pub fn shape_of(&self, model: &str) -> Option<Shape> {
        self.machines.iter().find(|row| row.models.iter().any(|m| m == model)).map(|row| row.shape)
    }
}

/// The shape of the computer whose model goes by this identifier, by the catalogue in use. Nothing when no
/// row lists it.
pub fn shape_of(model: &str) -> Option<Shape> {
    current().shape_of(model)
}

/// A model identifier as a system gives one: letters, then digits, a comma and digits.
fn identifier(model: &str) -> bool {
    let Some((head, minor)) = model.split_once(',') else { return false };
    let letters = head.bytes().take_while(u8::is_ascii_alphabetic).count();
    letters > 0 && letters < head.len() && head[letters..].bytes().all(|b| b.is_ascii_digit()) && !minor.is_empty() && minor.bytes().all(|b| b.is_ascii_digit())
}

/// Are the rows well formed? Each row's id is plain and its own; each identifier reads as one and is listed
/// once in the whole catalogue, so no identifier has two shapes.
pub(super) fn check_machines(rows: &[Machines]) -> Result<(), String> {
    for row in rows {
        if !form::plain(&row.id) || rows.iter().filter(|r| r.id == row.id).count() != 1 {
            return Err(format!("\"{}\" cannot be a machine row's id, or is there twice", row.id));
        }
        if row.models.is_empty() {
            return Err(format!("{}: lists no model", row.id));
        }
        for m in &row.models {
            if !identifier(m) {
                return Err(format!("{}: \"{m}\" is not a model identifier", row.id));
            }
            if rows.iter().flat_map(|r| r.models.iter()).filter(|o| *o == m).count() != 1 {
                return Err(format!("{m} is listed twice"));
            }
        }
    }
    Ok(())
}

/// The rows a later copy is taken with: every row this build has, as it has it, and the identifiers the copy
/// adds. An identifier this build knows keeps the shape it has here; the copy saying another is left out and
/// said. New identifiers in a row this build has go into it when the shape is the same.
pub(super) fn with_later(built: &[Machines], later: Vec<Machines>, left_out: &mut Vec<String>) -> Vec<Machines> {
    let mut rows = built.to_vec();
    for mut row in later {
        row.models.retain(|m| match built.iter().find(|b| b.models.contains(m)) {
            Some(b) if b.shape != row.shape => {
                left_out.push(format!("{}: {m} is a {:?} here", row.id, b.shape));
                false
            }
            Some(_) => false,
            None => true,
        });
        if row.models.is_empty() {
            continue;
        }
        match rows.iter_mut().find(|b| b.id == row.id) {
            Some(b) if b.shape == row.shape => row.models.into_iter().for_each(|m| if !b.models.contains(&m) { b.models.push(m) }),
            Some(_) => left_out.push(format!("{}: a row this build has with another shape", row.id)),
            None => rows.push(row),
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_shape_is_found_by_an_exact_identifier() {
        let c = built();
        for (model, shape) in [("MacBookAir10,1", Shape::Laptop), ("Mac14,7", Shape::Laptop), ("Mac16,10", Shape::Mini), ("Macmini9,1", Shape::Mini), ("Mac13,1", Shape::Studio), ("iMac21,1", Shape::Imac), ("Mac14,8", Shape::Pro), ("MacPro7,1", Shape::Pro)] {
            assert_eq!(c.shape_of(model), Some(shape), "{model}");
        }
    }

    #[test]
    fn an_identifier_no_row_lists_has_no_shape_however_close() {
        let c = built();
        for model in ["Mac99,1", "Mac16", "mac16,10", "Mac16,10 ", "Mac16,1O", "MacBookPro", "", "Macmini9,10"] {
            assert_eq!(c.shape_of(model), None, "{model:?}");
        }
    }

    #[test]
    fn an_identifier_listed_twice_is_refused() {
        let row = |id: &str, shape, models: &[&str]| Machines { id: id.into(), shape, models: models.iter().map(|m| m.to_string()).collect() };
        assert!(check_machines(&[row("a", Shape::Laptop, &["Mac1,1"]), row("b", Shape::Mini, &["Mac2,1"])]).is_ok());
        assert!(check_machines(&[row("a", Shape::Laptop, &["Mac1,1"]), row("b", Shape::Mini, &["Mac1,1"])]).unwrap_err().contains("twice"));
        assert!(check_machines(&[row("a", Shape::Laptop, &["Mac1,1", "Mac1,1"])]).unwrap_err().contains("twice"));
        assert!(check_machines(&[row("a", Shape::Laptop, &["Mac1"])]).is_err());
        assert!(check_machines(&[row("a", Shape::Laptop, &["Mac1,1"]), row("a", Shape::Mini, &["Mac2,1"])]).is_err());
        // the whole catalogue is held to it
        let mut c = read(BUILT_IN).unwrap();
        let first = c.machines[0].models[0].clone();
        c.machines[1].models.push(first);
        assert!(check(&c).unwrap_err().contains("twice"));
    }

    #[test]
    fn a_later_copy_adds_identifiers_and_never_changes_a_known_ones_shape() {
        let built = vec![Machines { id: "minis".into(), shape: Shape::Mini, models: vec!["Mac1,1".into()] }];
        let later = vec![
            Machines { id: "minis".into(), shape: Shape::Mini, models: vec!["Mac1,1".into(), "Mac1,2".into()] },
            Machines { id: "laptops".into(), shape: Shape::Laptop, models: vec!["Mac1,1".into(), "Mac3,1".into()] },
        ];
        let mut left_out = Vec::new();
        let rows = with_later(&built, later, &mut left_out);
        let c = |m: &str| rows.iter().find(|r| r.models.iter().any(|x| x == m)).map(|r| r.shape);
        assert_eq!((c("Mac1,1"), c("Mac1,2"), c("Mac3,1")), (Some(Shape::Mini), Some(Shape::Mini), Some(Shape::Laptop)));
        assert_eq!(left_out.len(), 1, "{left_out:?}");
        check_machines(&rows).unwrap();
    }

    #[test]
    fn a_signed_later_copy_may_bring_a_new_machine() {
        let mut doc: Value = serde_json::from_str(BUILT_IN).unwrap();
        doc["rev"] = (built().rev + 1).into();
        doc["expires"] = (crate::clock::now_secs() + 3600).into();
        doc["machines"].as_array_mut().unwrap().push(serde_json::json!({ "id": "later", "shape": "studio", "models": ["Mac99,1"] }));
        let (taken, _) = weigh(&doc.to_string(), crate::clock::now_secs(), built()).unwrap();
        let taken = taken.unwrap();
        assert_eq!(taken.shape_of("Mac99,1"), Some(Shape::Studio));
        assert_eq!(taken.shape_of("Mac16,10"), Some(Shape::Mini));
    }
}
