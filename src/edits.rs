use crate::{Result, project::validate_id};
use serde_json::{Value, json};
use std::collections::HashSet;

const MAX_SECONDS: f64 = 120.0;

pub fn edit(asset: &mut Value, command: &Value) -> Result<Value> {
    let id = &command["id"];
    match command["command"].as_str() {
        Some("asset_name") => {
            asset["name"] = name(&command["name"])?;
            Ok(json!({"name": asset["name"]}))
        }
        Some("track_add") => {
            check_id(id)?;
            let mut track = json!({"id": id, "name": id, "gain": 1, "pan": 0, "mute": false, "solo": false, "clips": []});
            track_mix(&mut track, command)?;
            add(&mut asset["tracks"], track)
        }
        Some("track_edit") => {
            let index = index(items(&asset["tracks"])?, id)?;
            let track = &mut asset["tracks"][index];
            track_mix(track, command)?;
            Ok(track.clone())
        }
        Some("clip_add") => {
            check_id(id)?;
            check_new_clip(asset, id)?;
            let track = index(items(&asset["tracks"])?, &command["track"])?;
            let mut clip = json!({"id": id, "name": id, "start": 0, "trim": 0, "gain": 1, "pan": 0, "fadeIn": 0, "fadeOut": 0, "loop": false, "trigger": null});
            update_clip(asset, &mut clip, command)?;
            add(&mut asset["tracks"][track]["clips"], clip)
        }
        Some("clip_edit" | "clip_remove" | "clip_split") => change_clip(asset, command, id),
        Some("control_add") => {
            check_id(id)?;
            let mut control = json!({"id": id, "name": name(default(&command["name"], id))?, "type": command["type"]});
            match control["type"].as_str() {
                Some("slider") => {
                    let min = number(default(&command["min"], &json!(0)), -1e9, 1e9, "min")?;
                    let max = number(default(&command["max"], &json!(1)), min + 1e-6, 1e9, "max")?;
                    let value = number(default(&command["value"], &json!(min)), min, max, "value")?;
                    control["min"] = json!(min);
                    control["max"] = json!(max);
                    control["value"] = json!(value);
                }
                Some("trigger") => {}
                _ => return Err("Expected slider or trigger".into()),
            }
            add(&mut asset["controls"], control)
        }
        Some("mapping_set") => set_mapping(asset, command),
        Some("preset_set") => {
            let preset = make_preset(asset, command)?;
            if asset["presets"].is_null() {
                asset["presets"] = json!([]);
            }
            let presets = items_mut(&mut asset["presets"])?;
            if let Some(existing) = presets.iter_mut().find(|entry| entry["id"] == *id) {
                *existing = preset.clone();
            } else {
                presets.push(preset.clone());
            }
            Ok(preset)
        }
        Some("preset_remove") => {
            let empty = json!([]);
            let index = index(items(default(&asset["presets"], &empty))?, id)?;
            items_mut(&mut asset["presets"])?.remove(index);
            Ok(json!({"id": id}))
        }
        _ => Err(format!(
            "Unknown command: {}",
            command["command"].as_str().unwrap_or("undefined")
        )
        .into()),
    }
}

pub fn add_melody(asset: &mut Value, command: &Value, source: &Value) -> Result<()> {
    let id = &source["id"];
    let name = default(&command["name"], &json!("Sung melody")).clone();
    add(&mut asset["sources"], source.clone())?;
    edit(
        asset,
        &json!({"command": "track_add", "id": id, "name": name}),
    )?;
    edit(
        asset,
        &json!({"command": "clip_add", "id": id, "track": id, "source": id, "name": name, "start": command["start"]}),
    )?;
    Ok(())
}

fn track_mix(track: &mut Value, command: &Value) -> Result<()> {
    track["name"] = name(default(&command["name"], &track["name"]))?;
    for (field, min, max) in [("gain", 0.0, 4.0), ("pan", -1.0, 1.0)] {
        set_number(track, command, field, min, max)?;
    }
    for field in ["mute", "solo"] {
        track[field] = json!(boolean(default(&command[field], &track[field]), field)?);
    }
    for field in ["lowPassHz", "highPassHz"] {
        if let Some(value) = command.get(field) {
            track[field] = if value.is_null() {
                Value::Null
            } else {
                json!(number(value, 30.0, 20_000.0, field)?)
            };
        }
    }

    Ok(())
}

fn update_clip(asset: &Value, clip: &mut Value, command: &Value) -> Result<()> {
    let source = find(
        &asset["sources"],
        default(&command["source"], &clip["source"]),
    )?;
    clip["source"] = source["id"].clone();
    clip["name"] = name(default(&command["name"], &clip["name"]))?;
    set_number(clip, command, "start", 0.0, MAX_SECONDS)?;
    set_number(clip, command, "trim", 0.0, scalar(&source["duration"]))?;

    let available = scalar(&source["duration"]) - scalar(&clip["trim"]);
    let length = number(
        default(
            &command["length"],
            default(&clip["length"], &json!(available)),
        ),
        1.0 / scalar(&source["rate"]),
        available + 1e-10,
        "length",
    )?;
    clip["length"] = json!(length);
    let duration = number(
        default(
            &command["duration"],
            default(&clip["duration"], &clip["length"]),
        ),
        0.001,
        MAX_SECONDS - scalar(&clip["start"]),
        "duration",
    )?;
    clip["duration"] = json!(duration);
    for (field, min, max) in [
        ("gain", 0.0, 4.0),
        ("pan", -1.0, 1.0),
        ("fadeIn", 0.0, duration),
        ("fadeOut", 0.0, duration),
    ] {
        set_number(clip, command, field, min, max)?;
    }
    clip["loop"] = json!(boolean(default(&command["loop"], &clip["loop"]), "loop")?);
    if let Some(trigger) = command.get("trigger") {
        clip["trigger"] = trigger.clone();
    }
    if !clip["trigger"].is_null()
        && find(&asset["controls"], &clip["trigger"])?["type"] != "trigger"
    {
        return Err("Expected a trigger control".into());
    }

    Ok(())
}

fn change_clip(asset: &mut Value, command: &Value, id: &Value) -> Result<Value> {
    let (track, position) =
        clip_position(asset, id).ok_or_else(|| format!("Unknown clip: {}", display(id)))?;
    let mut clip = asset["tracks"][track]["clips"][position].clone();
    match command["command"].as_str() {
        Some("clip_remove") => {
            items_mut(&mut asset["tracks"][track]["clips"])?.remove(position);
            items_mut(&mut asset["mappings"])?.retain(|mapping| mapping["clip"] != *id);
            Ok(json!({"id": id}))
        }
        Some("clip_split") => {
            let pitch_mapped = items(&asset["mappings"])?
                .iter()
                .any(|mapping| mapping["clip"] == *id && mapping["target"] == "pitch");
            if clip["loop"] == true || !clip["trigger"].is_null() || pitch_mapped {
                return Err("Split requires a fixed, untriggered clip".into());
            }

            let duration = scalar(&clip["duration"]);
            let length = scalar(&clip["length"]);
            let fade_in = scalar(&clip["fadeIn"]);
            let fade_out = scalar(&clip["fadeOut"]);
            let offset = scalar(&command["time"]) - scalar(&clip["start"]);
            let first = number(
                &json!(offset),
                0.001,
                duration.min(length) - 0.001,
                "split offset",
            )?;
            if first < fade_in || first > duration - fade_out {
                return Err("Split outside the clip's fades".into());
            }

            let new_id = &command["newId"];
            check_id(new_id)?;
            check_new_clip(asset, new_id)?;
            let mut second = clip.clone();
            second["id"] = new_id.clone();
            update_clip(
                asset,
                &mut second,
                &json!({
                    "start": command["time"], "trim": scalar(&clip["trim"]) + first, "length": length - first,
                    "duration": duration - first, "fadeIn": 0, "fadeOut": fade_out.min(duration - first)
                }),
            )?;
            update_clip(
                asset,
                &mut clip,
                &json!({"length": first, "duration": first, "fadeIn": fade_in.min(first), "fadeOut": 0}),
            )?;

            let clips = items_mut(&mut asset["tracks"][track]["clips"])?;
            clips[position] = clip.clone();
            clips.push(second.clone());
            let mappings = items_mut(&mut asset["mappings"])?;
            let duplicated: Vec<_> = mappings
                .iter()
                .filter(|mapping| mapping["clip"] == *id)
                .map(|mapping| {
                    let mut mapping = mapping.clone();
                    mapping["clip"] = new_id.clone();
                    mapping
                })
                .collect();
            mappings.extend(duplicated);
            Ok(json!({"first": clip, "second": second}))
        }
        _ => {
            update_clip(asset, &mut clip, command)?;
            let destination = &command["track"];
            let should_move = !destination.is_null()
                && destination != ""
                && destination != &asset["tracks"][track]["id"];
            if should_move {
                let destination = index(items(&asset["tracks"])?, destination)?;
                items_mut(&mut asset["tracks"][track]["clips"])?.remove(position);
                items_mut(&mut asset["tracks"][destination]["clips"])?.push(clip.clone());
            } else {
                asset["tracks"][track]["clips"][position] = clip.clone();
            }
            Ok(clip)
        }
    }
}

fn set_mapping(asset: &mut Value, command: &Value) -> Result<Value> {
    let control = find(&asset["controls"], &command["control"])?;
    if control["type"] != "slider" {
        return Err("Mappings require a slider".into());
    }

    let clip = &command["clip"];
    if clip_position(asset, clip).is_none() {
        return Err(format!("Unknown ID: {}", display(clip)).into());
    }

    let target = &command["target"];
    if !matches!(target.as_str(), Some("gain" | "pitch")) {
        return Err("Map a slider to gain or pitch".into());
    }

    let combine = boolean(default(&command["combine"], &json!(false)), "combine")?;
    let linear = json!("linear");
    let curve = default(&command["curve"], &linear);
    if !matches!(curve.as_str(), Some("linear" | "equal-power")) {
        return Err("Expected linear or equal-power curve".into());
    }
    if (combine || curve != "linear") && target != "gain" {
        return Err("Combined mappings and equal-power curves require gain".into());
    }

    let min = scalar(&control["min"]);
    let max = scalar(&control["max"]);
    let input_min = number(
        default(&command["inputMin"], &control["min"]),
        min,
        max,
        "inputMin",
    )?;
    let input_max = number(
        default(&command["inputMax"], &control["max"]),
        input_min,
        max,
        "inputMax",
    )?;
    if input_max <= input_min {
        return Err("inputMax must be greater than inputMin".into());
    }

    let minimum = if target == "pitch" { 0.125 } else { 0.0 };
    let mut mapping = json!({
        "control": control["id"], "clip": clip, "target": target, "inputMin": input_min, "inputMax": input_max,
        "min": number(&command["min"], minimum, 4.0, "min")?, "max": number(&command["max"], minimum, 4.0, "max")?
    });
    if curve != "linear" {
        mapping["curve"] = curve.clone();
    }

    let mappings = items_mut(&mut asset["mappings"])?;
    mappings.retain(|entry| {
        entry["clip"] != *clip
            || entry["target"] != *target
            || (combine && entry["control"] != mapping["control"])
    });
    mappings.push(mapping.clone());
    Ok(mapping)
}

fn make_preset(asset: &Value, command: &Value) -> Result<Value> {
    let id = &command["id"];
    check_id(id)?;
    let name = name(default(&command["name"], id))?;
    let seconds = number(&command["seconds"], 0.001, MAX_SECONDS, "seconds")?;
    let sustain = boolean(default(&command["sustain"], &json!(false)), "sustain")?;
    let empty = json!([]);
    let lanes = default(&command["lanes"], &empty)
        .as_array()
        .filter(|items| items.len() <= 16)
        .ok_or("Expected up to 16 slider lanes")?;
    let triggers = default(&command["triggers"], &empty)
        .as_array()
        .filter(|items| items.len() <= 1000)
        .ok_or("Expected up to 1000 trigger events")?;

    let mut seen = HashSet::new();
    let mut slider_lanes = Vec::new();
    for lane in lanes {
        let control = find(&asset["controls"], &lane["control"])?;
        if control["type"] != "slider" || !seen.insert(display(&control["id"])) {
            return Err("Expected a distinct slider for each lane".into());
        }
        let points = lane["points"]
            .as_array()
            .filter(|items| (1..=1000).contains(&items.len()))
            .ok_or("Each slider lane needs 1 to 1000 keyframes")?;
        let mut previous = -1.0;
        let mut keyframes = Vec::new();
        for point in points {
            let time = number(&point["time"], 0.0, seconds, "keyframe time")?;
            if time <= previous {
                return Err("Keyframe times must increase".into());
            }

            let value = number(
                &point["value"],
                scalar(&control["min"]),
                scalar(&control["max"]),
                "keyframe value",
            )?;
            previous = time;
            keyframes.push(json!({"time": time, "value": value}));
        }
        slider_lanes.push(json!({"control": control["id"], "points": keyframes}));
    }

    let mut trigger_events = Vec::new();
    for event in triggers {
        let control = find(&asset["controls"], &event["control"])?;
        if control["type"] != "trigger" {
            return Err("Expected a trigger control".into());
        }
        let time = number(&event["time"], 0.0, seconds, "trigger time")?;
        if time >= seconds {
            return Err("Trigger must fire before the preset ends".into());
        }
        trigger_events.push(json!({"control": control["id"], "time": time}));
    }

    Ok(
        json!({"id": id, "name": name, "seconds": seconds, "sustain": sustain, "lanes": slider_lanes, "triggers": trigger_events}),
    )
}

pub fn add(list: &mut Value, item: Value) -> Result<Value> {
    let list = items_mut(list)?;
    if list.iter().any(|entry| entry["id"] == item["id"]) {
        return Err(format!("ID already exists: {}", display(&item["id"])).into());
    }
    list.push(item.clone());

    Ok(item)
}

fn clip_position(asset: &Value, id: &Value) -> Option<(usize, usize)> {
    asset["tracks"]
        .as_array()?
        .iter()
        .enumerate()
        .find_map(|(track, entry)| {
            let clip = entry["clips"]
                .as_array()?
                .iter()
                .position(|item| item["id"] == *id)?;
            Some((track, clip))
        })
}

fn check_new_clip(asset: &Value, id: &Value) -> Result<()> {
    if clip_position(asset, id).is_some() {
        return Err("Clip ID already exists".into());
    }
    Ok(())
}

fn check_id(id: &Value) -> Result<()> {
    validate_id(id.as_str().unwrap_or_default())
}

fn items(list: &Value) -> Result<&[Value]> {
    list.as_array()
        .map(Vec::as_slice)
        .ok_or_else(|| "Invalid asset collection".into())
}

fn items_mut(list: &mut Value) -> Result<&mut Vec<Value>> {
    list.as_array_mut()
        .ok_or_else(|| "Invalid asset collection".into())
}

fn index(list: &[Value], id: &Value) -> Result<usize> {
    list.iter()
        .position(|entry| entry["id"] == *id)
        .ok_or_else(|| format!("Unknown ID: {}", display(id)).into())
}

fn find<'a>(list: &'a Value, id: &Value) -> Result<&'a Value> {
    let list = items(list)?;
    let index = index(list, id)?;
    Ok(&list[index])
}

fn default<'a>(value: &'a Value, fallback: &'a Value) -> &'a Value {
    if value.is_null() { fallback } else { value }
}

fn display(value: &Value) -> String {
    value
        .as_str()
        .map_or_else(|| value.to_string(), str::to_owned)
}

fn name(value: &Value) -> Result<Value> {
    if !value.as_str().is_some_and(|text| !text.trim().is_empty()) {
        return Err("Expected a name".into());
    }
    Ok(value.clone())
}

fn boolean(value: &Value, field: &str) -> Result<bool> {
    value
        .as_bool()
        .ok_or_else(|| format!("{field} must be true or false").into())
}

fn scalar(value: &Value) -> f64 {
    value.as_f64().unwrap_or(f64::NAN)
}

fn number(value: &Value, min: f64, max: f64, field: &str) -> Result<f64> {
    let value = scalar(value);
    if !(min..=max).contains(&value) {
        return Err(format!("{field} must be between {min} and {max}").into());
    }
    Ok(value)
}

fn set_number(
    document: &mut Value,
    command: &Value,
    field: &str,
    min: f64,
    max: f64,
) -> Result<()> {
    let value = number(default(&command[field], &document[field]), min, max, field)?;
    document[field] = json!(value);
    Ok(())
}
