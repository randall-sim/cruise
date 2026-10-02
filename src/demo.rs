//! The explicitly requested empty-workspace example shown by the frontend.
use crate::{
    core,
    store::{Result, Store, arg, array, array_mut, now},
    study,
};
use serde_json::{Value, json};

pub fn seed(store: &Store) -> Result<Value> {
    if !array(&store.read()?, "courses").is_empty() {
        return Err("Demo requires an empty workspace".into());
    }
    let fixture: Value =
        serde_json::from_str(include_str!("demo.json")).map_err(|e| e.to_string())?;
    let mut courses = Vec::new();
    for fields in array(&fixture, "courses") {
        let mut course = core::call(store, "create_course", fields.clone())?;
        course["demo"] = json!(true);
        store.mutate(|state| {
            *array_mut(state, "courses")?
                .iter_mut()
                .find(|item| item["id"] == course["id"])
                .ok_or("Demo course is missing")? = course.clone();
            store.write(
                &format!("courses/{}/course.json", arg(&course, "id")?),
                &serde_json::to_vec_pretty(&course).map_err(|e| e.to_string())?,
            )
        })?;
        courses.push(course);
    }
    let mut input = fixture["lecture"].clone();
    input["courseId"] = courses[0]["id"].clone();
    input["date"] = json!(&now()[..10]);
    let lecture = core::call(store, "lecture.import", input)?;
    let job = study::create_job(
        store,
        json!({"kind":"lecture","courseId":courses[0]["id"],"lectureId":lecture["id"],"prompt":"Explain this fictional example lecture."}),
    )?;
    let output = serde_json::to_string(&fixture["guide"])
        .map_err(|e| e.to_string())?
        .replace("DEMO_LECTURE", arg(&lecture, "id")?);
    study::complete_job(
        store,
        arg(&job, "id")?,
        serde_json::from_str(&output).map_err(|e| e.to_string())?,
    )?;
    Ok(courses.remove(0))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_demo_has_complete_review_modes_and_refuses_existing_content() {
        let root = std::env::temp_dir().join(format!("cruise-demo-{}", crate::store::id()));
        let store = Store::new(root.clone());
        let course = seed(&store).unwrap();
        assert_eq!(course["demo"], true);
        let before = store.read().unwrap();
        assert_eq!(array(&before, "courses").len(), 3);
        assert!(
            array(&before, "courses")
                .iter()
                .all(|course| course["demo"] == true)
        );
        assert_eq!(before["lectures"][0]["status"], "ready");
        let guide = &before["lectures"][0]["guide"];
        assert_eq!(array(guide, "questions").len(), 8);
        assert!(
            array(guide, "sections")
                .iter()
                .all(|page| page["fastMarkdown"].as_str().unwrap().lines().count() >= 3)
        );
        assert!(seed(&store).unwrap_err().contains("empty workspace"));
        assert_eq!(store.read().unwrap(), before);
        std::fs::remove_dir_all(root).unwrap();
    }
}
