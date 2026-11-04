//! Participant management for remote control sessions.
//!
//! This module provides the Participant and ParticipantsManager types for managing
//! per-participant state including drawing and cursor rendering.

use crate::utils::geometry::Position;
use hopp_marks::colours::{self, Colours};
use hopp_marks::stamps::{Stamp, StampInput, StampMessage, Stamps};
use iced::widget::canvas::{Frame, Geometry};
use iced::{Rectangle, Renderer};
use std::collections::HashMap;
use std::time::Instant;
use thiserror::Error;

#[path = "draw.rs"]
mod draw;
use draw::Draw;

#[path = "cursor.rs"]
pub mod cursor;
use cursor::Cursor;

// Re-export CursorMode for external use
pub use cursor::CursorMode;

#[path = "stamp.rs"]
mod stamp;
use stamp::{StampKey, StampVisual};

use crate::room_service::DrawingMode;

/// How the manager knows you, whoever you are in the call.
const LOCAL_IDENTITY: &str = "local";
/// What your own stamps say they're from.
const YOUR_STAMP_NAME: &str = "You";
/// What a stamp says it's from when its sender's name isn't known.
const UNKNOWN_STAMP_NAME: &str = "Someone";

/// Errors that can occur during participant management.
#[derive(Error, Debug)]
pub enum ParticipantError {
    #[error("Participant already exists: {0}")]
    AlreadyExists(String),
    #[error("Couldn't render the cursor for {0}")]
    CursorRender(String),
}

/// Generates a unique visible name based on the full name and existing names.
///
/// # Algorithm
/// - For "John Smith" with no conflicts: returns "John"
/// - If "John" exists: tries "John S", then "John Sm", etc.
/// - If full name exists: adds numbers "John Smith2", "John Smith3", etc.
fn generate_unique_visible_name(name: &str, used_names: &[String]) -> String {
    let parts: Vec<&str> = name.split_whitespace().collect();
    let first_name = parts.first().unwrap_or(&name);

    // Try progressively longer candidates
    let candidates = if parts.len() > 1 {
        let last_name = parts[1];
        let mut candidates = vec![first_name.to_string()];

        // Add candidates with increasing characters from last name
        for i in 1..=last_name.chars().count() {
            let partial_last_name: String = last_name.chars().take(i).collect();
            candidates.push(format!("{first_name} {partial_last_name}"));
        }
        candidates
    } else {
        vec![first_name.to_string()]
    };

    // Find first unused candidate
    for candidate in candidates.iter() {
        if !used_names.contains(candidate) {
            return candidate.clone();
        }
    }

    // Fall back to numbering
    let base = candidates.last().unwrap().clone();
    for num in 2.. {
        let candidate = format!("{base}{num}");
        if !used_names.contains(&candidate) {
            return candidate;
        }
    }

    unreachable!()
}

/// Represents a participant in a remote control session.
///
/// Each participant has their own drawing state, cursor, and color.
#[derive(Debug)]
pub struct Participant {
    draw: Draw,
    cursor: Cursor,
    rgb: [u8; 3],
}

impl Participant {
    /// Creates a new participant with the given color, name and auto-clear setting.
    ///
    /// # Arguments
    /// * `rgb` - Color of the participant's drawings and cursor
    /// * `name` - Display name for the participant's cursor
    /// * `auto_clear` - Whether to automatically clear paths after 3 seconds
    /// * `follows_instantly` - Whether the cursor skips playback, because it's your own
    pub fn new(
        rgb: [u8; 3],
        name: &str,
        auto_clear: bool,
        initial_drawing_mode: DrawingMode,
        follows_instantly: bool,
    ) -> Result<Self, ParticipantError> {
        Ok(Self {
            draw: Draw::new(&colours::hex(rgb), auto_clear, initial_drawing_mode),
            cursor: Cursor::new(rgb, name, follows_instantly)
                .ok_or_else(|| ParticipantError::CursorRender(name.to_string()))?,
            rgb,
        })
    }

    /// Changes the participant's color, including ink they've already drawn.
    fn set_colour(&mut self, rgb: [u8; 3]) {
        if self.rgb == rgb {
            return;
        }
        self.rgb = rgb;
        self.draw.set_color(&colours::hex(rgb));
        self.cursor.set_colour(rgb);
    }

    /// Returns a reference to the participant's Draw instance.
    pub fn draw(&self) -> &Draw {
        &self.draw
    }

    /// Returns a mutable reference to the participant's Draw instance.
    pub fn draw_mut(&mut self) -> &mut Draw {
        &mut self.draw
    }

    /// Returns a reference to the participant's cursor.
    pub fn cursor(&self) -> &Cursor {
        &self.cursor
    }

    /// Returns a mutable reference to the participant's cursor.
    pub fn cursor_mut(&mut self) -> &mut Cursor {
        &mut self.cursor
    }
}

/// A participant's color: the one the call agreed on, or else the one they'd prefer.
fn colour_of(call_colours: &Colours, local_identity: Option<&str>, identity: &str) -> [u8; 3] {
    let person = match local_identity {
        Some(local_identity) if identity == LOCAL_IDENTITY => local_identity,
        _ => identity,
    };
    call_colours
        .get(person)
        .unwrap_or_else(|| colours::colour(person))
}

/// Manager that owns Participant objects mapped by participant identity.
///
/// Each participant gets their own Participant instance with their color, drawing state,
/// and cursor.
#[derive(Debug, Default)]
pub struct ParticipantsManager {
    participants: HashMap<String, Participant>,
    /// Everyone's colors in this call, worked out the same way on every machine.
    call_colours: Colours,
    /// Who you are in this call, for your own color.
    local_identity: Option<String>,
    stamps: Stamps,
    /// The note you're typing over someone's screen, under your pointer.
    note_ghost: Option<Stamp>,
    stamp_visuals: HashMap<StampKey, StampVisual>,
}

impl ParticipantsManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Gives everyone the colors worked out from who's in the call, so each person has the
    /// same one on every machine. A clash can move someone's color as people come and go.
    pub fn set_call_people(&mut self, local_identity: Option<&str>, people: &[String]) {
        self.local_identity = local_identity.map(str::to_owned);
        self.call_colours =
            Colours::for_call(people.iter().map(String::as_str).chain(local_identity));
        for (identity, participant) in &mut self.participants {
            participant.set_colour(colour_of(&self.call_colours, local_identity, identity));
        }
        self.refresh_stamp_visuals();
    }

    /// Does what you asked with your stamps, and returns what everyone else needs to hear.
    pub fn stamp_input(&mut self, input: StampInput) -> Vec<StampMessage> {
        let messages = self.stamps.apply(input, YOUR_STAMP_NAME, Instant::now());
        self.refresh_stamp_visuals();
        messages
    }

    /// A stamp message from someone else in the call.
    pub fn receive_stamp(&mut self, sender: &str, message: StampMessage) {
        let name = self.stamp_name(sender);
        self.stamps.receive(sender, &name, message, Instant::now());
        self.refresh_stamp_visuals();
    }

    /// You've a stamp taking words, so keys go into it.
    pub fn is_stamp_typing(&self) -> bool {
        self.stamps.is_typing()
    }

    /// Your stamp taking words hears what you say.
    pub fn is_stamp_hearing(&self) -> bool {
        self.stamps.is_hearing()
    }

    /// The note you're typing and where your pointer is, if it's over the screen.
    pub fn set_note_ghost(&mut self, note: Option<((f64, f64), &str)>) {
        self.note_ghost = note.map(|(point, text)| {
            self.stamps
                .ghost(point, text, YOUR_STAMP_NAME, Instant::now())
        });
        self.refresh_stamp_visuals();
    }

    pub fn is_stamp_animating(&self) -> bool {
        self.stamps.animating(Instant::now())
    }

    /// Their connection sending the stamp isn't always the one that has their cursor.
    fn stamp_name(&self, sender: &str) -> String {
        let person = colours::person(sender);
        self.participants
            .iter()
            .find(|(identity, _)| colours::person(identity) == person)
            .map_or(UNKNOWN_STAMP_NAME.to_owned(), |(_, participant)| {
                participant.cursor().visible_name().trim().to_owned()
            })
    }

    /// Brings what's drawn for each stamp up to date. Rendering and measuring happen here
    /// and not while drawing, so the only stamps that cost anything are ones that changed.
    fn refresh_stamp_visuals(&mut self) {
        let Self {
            stamps,
            note_ghost,
            stamp_visuals,
            call_colours,
            local_identity,
            ..
        } = self;
        stamp_visuals.retain(|(owner, id), _| {
            stamps
                .iter()
                .chain(note_ghost.iter())
                .any(|stamp| stamp.owner == *owner && stamp.id == *id)
        });
        for stamp in stamps.iter().chain(note_ghost.iter()) {
            let identity = if stamp.is_mine() {
                LOCAL_IDENTITY
            } else {
                stamp.owner.as_str()
            };
            let rgb = colour_of(call_colours, local_identity.as_deref(), identity);
            let key = (stamp.owner.clone(), stamp.id);
            if let Some(visual) = stamp_visuals.get_mut(&key) {
                visual.update(stamp, rgb);
            } else if let Some(visual) = StampVisual::new(stamp, rgb) {
                stamp_visuals.insert(key, visual);
            } else {
                log::warn!("refresh_stamp_visuals: couldn't render stamp {}", stamp.id);
            }
        }
    }

    /// Adds a new participant with their color.
    ///
    /// # Arguments
    /// * `identity` - Identity for the participant
    /// * `name` - Full name of the participant (will be made unique)
    /// * `auto_clear` - Whether to automatically clear paths after 3 seconds
    ///
    pub fn add_participant(
        &mut self,
        identity: String,
        name: &str,
        auto_clear: bool,
        initial_drawing_mode: DrawingMode,
    ) -> Result<(), ParticipantError> {
        // Check if participant already exists
        if self.participants.contains_key(&identity) {
            return Err(ParticipantError::AlreadyExists(identity));
        }

        let rgb = colour_of(
            &self.call_colours,
            self.local_identity.as_deref(),
            &identity,
        );

        let used_names: Vec<String> = self
            .participants
            .values()
            .map(|p| p.cursor().visible_name().to_string())
            .collect();
        let visible_name = generate_unique_visible_name(name, &used_names);

        log::info!(
            "ParticipantsManager::add_participant: identity={} color={} auto_clear={}",
            identity,
            colours::hex(rgb),
            auto_clear
        );

        let follows_instantly = identity == LOCAL_IDENTITY;
        self.participants.insert(
            identity,
            Participant::new(
                rgb,
                &visible_name,
                auto_clear,
                initial_drawing_mode,
                follows_instantly,
            )?,
        );
        Ok(())
    }

    /// Removes all participants and forgets the call's colors.
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn clear_draw_caches(&mut self) {
        for participant in self.participants.values_mut() {
            participant.draw_mut().clear_cache();
        }
    }

    pub fn drawing_modes(&self) -> Vec<(String, DrawingMode)> {
        self.participants
            .iter()
            .map(|(identity, participant)| (identity.clone(), participant.draw().mode()))
            .collect()
    }

    /// Removes a participant and their data.
    pub fn remove_participant(&mut self, identity: &str) {
        log::info!(
            "ParticipantsManager::remove_participant: identity={}",
            identity
        );
        self.stamps.sender_left(identity);
        if self.participants.remove(identity).is_none() {
            log::warn!(
                "ParticipantsManager::remove_participant: participant {} not found",
                identity
            );
        }
    }

    /// Sets the drawing mode for a specific participant.
    pub fn set_drawing_mode(&mut self, identity: &str, mode: DrawingMode) {
        log::debug!(
            "ParticipantsManager::set_drawing_mode: identity={} mode={:?}",
            identity,
            mode
        );
        if let Some(participant) = self.participants.get_mut(identity) {
            participant.draw_mut().set_mode(mode);
        } else {
            log::warn!(
                "ParticipantsManager::set_drawing_mode: participant {} not found",
                identity
            );
        }
    }

    /// Starts a new drawing path for a participant.
    ///
    /// # Returns
    /// Whether this redraws a path that was just cleared, in which case the pen hasn't moved here.
    pub fn draw_start(&mut self, identity: &str, point: Position, path_id: u64) -> bool {
        log::debug!(
            "ParticipantsManager::draw_start: identity={} point={:?} path_id={}",
            identity,
            point,
            path_id
        );
        if let Some(participant) = self.participants.get_mut(identity) {
            participant.draw_mut().start_path(path_id, point)
        } else {
            log::warn!(
                "ParticipantsManager::draw_start: participant {} not found",
                identity
            );
            false
        }
    }

    /// Adds a point to the current drawing path for a participant.
    pub fn draw_add_point(&mut self, identity: &str, point: Position) {
        log::debug!(
            "ParticipantsManager::draw_add_point: identity={} point={:?}",
            identity,
            point
        );
        if let Some(participant) = self.participants.get_mut(identity) {
            participant.draw_mut().add_point(point);
        } else {
            log::warn!(
                "ParticipantsManager::draw_add_point: participant {} not found",
                identity
            );
        }
    }

    /// Reduces a participant's current path to a straight line ending at `end`.
    pub fn draw_straighten(&mut self, identity: &str, end: Position) {
        if let Some(participant) = self.participants.get_mut(identity) {
            participant.draw_mut().straighten_in_progress(end);
        }
    }

    /// Takes back a participant's most recently finished path, returning its ID.
    pub fn draw_undo_last(&mut self, identity: &str) -> Option<u64> {
        self.participants
            .get_mut(identity)
            .and_then(|participant| participant.draw_mut().undo_last_path())
    }

    /// Ends the current drawing path for a participant.
    pub fn draw_end(&mut self, identity: &str, point: Position) {
        log::debug!(
            "ParticipantsManager::draw_end: identity={} point={:?}",
            identity,
            point
        );
        if let Some(participant) = self.participants.get_mut(identity) {
            participant.draw_mut().add_point(point);
            participant.draw_mut().finish_path();
        } else {
            log::warn!(
                "ParticipantsManager::draw_end: participant {} not found",
                identity
            );
        }
    }

    /// Clears a specific drawing path for a participant.
    pub fn draw_clear_path(&mut self, identity: &str, path_id: u64) {
        log::debug!(
            "ParticipantsManager::draw_clear_path: identity={} path_id={}",
            identity,
            path_id
        );
        if let Some(participant) = self.participants.get_mut(identity) {
            participant.draw_mut().clear_path(path_id);
        } else {
            log::warn!(
                "ParticipantsManager::draw_clear_path: participant {} not found",
                identity
            );
        }
    }

    /// Clears all drawing paths for a participant.
    pub fn draw_clear_all_paths(&mut self, identity: &str) {
        log::info!(
            "ParticipantsManager::draw_clear_all_paths: identity={}",
            identity
        );
        if let Some(participant) = self.participants.get_mut(identity) {
            participant.draw_mut().clear();
        } else {
            log::warn!(
                "ParticipantsManager::draw_clear_all_paths: participant {} not found",
                identity
            );
        }
    }

    /// Updates auto-clear for all participants and returns removed path IDs.
    ///
    /// This should be called periodically to expire old paths for participants
    /// with auto_clear enabled.
    ///
    /// # Returns
    /// A vector of removed path IDs
    pub fn update_auto_clear(&mut self) -> Vec<u64> {
        let mut removed_path_ids = Vec::new();
        for participant in self.participants.values_mut() {
            removed_path_ids.extend(participant.draw_mut().clear_expired_paths());
        }
        removed_path_ids
    }

    /// Whether any participant's ink is animating and needs frames even with no input.
    pub fn is_ink_animating(&self) -> bool {
        self.participants
            .values()
            .any(|participant| participant.draw().is_animating())
    }

    /// Sets the cursor position for a specific participant.
    pub fn set_cursor_position(&mut self, identity: &str, position: Option<Position>) {
        if let Some(participant) = self.participants.get_mut(identity) {
            participant.cursor_mut().set_position(position);
        }
    }

    /// Sets the cursor mode for a participant.
    pub fn set_cursor_mode(&mut self, identity: &str, mode: CursorMode) {
        if let Some(participant) = self.participants.get_mut(identity) {
            participant.cursor_mut().set_mode(mode);
        }
    }

    /// Whether any cursor is moving or fading and needs frames even with no input.
    pub fn is_cursor_animating(&self) -> bool {
        let now = Instant::now();
        self.participants
            .values()
            .any(|participant| participant.cursor().is_animating(now))
    }

    /// Hides cursors that have rested long enough to have faded out. A pen held still in the
    /// middle of a stroke is still there.
    pub fn hide_inactive_cursors(&mut self) {
        let now = Instant::now();
        for participant in self.participants.values_mut() {
            if participant.draw().pen_tip().is_none() {
                participant.cursor_mut().hide_if_gone(now);
            }
        }
    }

    /// Renders all participants' drawings and cursors.
    ///
    /// # Returns
    /// A vector of Geometry objects representing all rendered content
    pub fn draw(
        &self,
        renderer: &Renderer,
        bounds: Rectangle,
        translate: &dyn Fn(Position) -> Position,
    ) -> Vec<Geometry> {
        let mut geometries = Vec::with_capacity(self.participants.len() + 1);

        // Collect cached completed geometries from each participant's Draw
        for participant in self.participants.values() {
            geometries.push(
                participant
                    .draw()
                    .draw_completed(renderer, bounds, translate),
            );
        }

        // Draw all in-progress paths into a single frame
        let mut in_progress_frame = Frame::new(renderer, bounds.size());
        for participant in self.participants.values() {
            participant
                .draw()
                .draw_in_progress_to_frame(&mut in_progress_frame, translate);
        }

        let now = Instant::now();
        stamp::paint_stamps(
            &mut in_progress_frame,
            self.stamps.iter().chain(self.note_ghost.iter()),
            &self.stamp_visuals,
            translate,
            now,
        );
        for participant in self.participants.values() {
            participant.cursor().draw(
                &mut in_progress_frame,
                translate,
                participant.draw().pen_tip(),
                now,
            );
        }
        geometries.push(in_progress_frame.into_geometry());

        geometries
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two machines in a call: what one does with stamps reaches the other as they should see it.
    #[test]
    fn stamps_get_from_one_machine_to_another() {
        let sender = "room:r:usr_mira:video";
        let mut mine = ParticipantsManager::new();
        let mut theirs = ParticipantsManager::new();
        theirs
            .add_participant(sender.to_owned(), "Mira Jones", false, DrawingMode::Any)
            .unwrap();

        let mut send = |manager: &mut ParticipantsManager, input| {
            for message in manager.stamp_input(input) {
                // Over the wire, as it really goes.
                let packet = serde_json::to_vec(&message).unwrap();
                let received = StampMessage::from_packet(&packet).expect("a good packet");
                theirs.receive_stamp(sender, received);
            }
        };
        send(&mut mine, StampInput::Place { x: 0.25, y: 0.75 });
        send(&mut mine, StampInput::Typed("Blue".into()));

        let (ours, seen) = (
            mine.stamps.iter().next().unwrap(),
            theirs.stamps.iter().next().unwrap(),
        );
        assert_eq!(ours.name, YOUR_STAMP_NAME);
        assert_eq!(ours.owner, "");
        assert_eq!(seen.name, "Mira", "their first name, as on their cursor");
        assert_eq!(seen.owner, sender);
        assert_eq!(seen.point, (0.25, 0.75));
        assert_eq!(seen.text, "Blue");
        assert!(seen.listening);
        assert_eq!(mine.stamp_visuals.len(), 1);
        assert_eq!(theirs.stamp_visuals.len(), 1);
        assert!(theirs.is_stamp_animating());

        // They leave without finishing: it stops waiting for them, and stays.
        theirs.remove_participant(sender);
        let seen = theirs.stamps.iter().next().unwrap();
        assert!(!seen.listening);
        assert_eq!(seen.text, "Blue");
    }

    #[test]
    fn a_stamp_from_someone_unknown_is_still_named() {
        let mut manager = ParticipantsManager::new();
        let place = StampMessage::StampPlace {
            id: 1,
            x: 0.5,
            y: 0.5,
        };
        manager.receive_stamp("room:r:usr_new:video", place);
        assert_eq!(
            manager.stamps.iter().next().unwrap().name,
            UNKNOWN_STAMP_NAME
        );
    }

    /// Their camera connection knows their name, even if it isn't the one that sent the stamp.
    #[test]
    fn a_stamp_finds_its_name_from_any_of_their_connections() {
        let mut manager = ParticipantsManager::new();
        manager
            .add_participant(
                identity("usr_sam", "audio"),
                "Sam Taylor",
                false,
                DrawingMode::Any,
            )
            .unwrap();
        let place = StampMessage::StampPlace {
            id: 1,
            x: 0.5,
            y: 0.5,
        };
        manager.receive_stamp(&identity("usr_sam", "video"), place);
        assert_eq!(manager.stamps.iter().next().unwrap().name, "Sam");
    }

    #[test]
    fn stamp_visuals_keep_up() {
        let mut manager = ParticipantsManager::new();
        manager.stamp_input(StampInput::Place { x: 0.5, y: 0.5 });
        let key = |manager: &ParticipantsManager| {
            let stamp = manager.stamps.iter().next().unwrap();
            (stamp.owner.clone(), stamp.id)
        };
        let id = key(&manager);
        assert!(manager.stamp_visuals.contains_key(&id));

        // Cancelled, then gone after its fade: the next thing you do lets go of what it drew.
        manager.stamp_input(StampInput::Cancel);
        assert!(manager.stamp_visuals.contains_key(&id), "still fading");
        std::thread::sleep(std::time::Duration::from_millis(280));
        manager.stamp_input(StampInput::Finish);
        assert!(manager.stamp_visuals.is_empty());
    }

    /// Two user ids that want the same color, so one of them has to move.
    fn clashing_people() -> (String, String) {
        let ids: Vec<String> = (0..50).map(|i| format!("usr_{i}")).collect();
        for first in &ids {
            for second in &ids {
                if first != second && colours::colour(first) == colours::colour(second) {
                    return (first.clone(), second.clone());
                }
            }
        }
        unreachable!("fifty people and eight colors");
    }

    fn identity(person: &str, connection: &str) -> String {
        format!("room:r:{person}:{connection}")
    }

    /// One machine in the call: `me`, looking at everyone else.
    fn machine(me: &str, others: &[&str]) -> ParticipantsManager {
        let mut manager = ParticipantsManager::new();
        manager
            .add_participant(LOCAL_IDENTITY.to_string(), "Me ", true, DrawingMode::Any)
            .unwrap();
        let others: Vec<String> = others
            .iter()
            .map(|person| identity(person, "video"))
            .collect();
        for other in &others {
            manager
                .add_participant(other.clone(), "Someone", false, DrawingMode::Any)
                .unwrap();
        }
        manager.set_call_people(Some(&identity(me, "audio")), &others);
        manager
    }

    fn colour_on(manager: &ParticipantsManager, identity: &str) -> [u8; 3] {
        manager.participants[identity].rgb
    }

    #[test]
    fn a_person_has_the_same_colour_on_every_machine() {
        let (first, second) = clashing_people();
        let on_firsts = machine(&first, &[&second]);
        let on_seconds = machine(&second, &[&first]);

        let first_on_seconds = colour_on(&on_seconds, &identity(&first, "video"));
        let second_on_firsts = colour_on(&on_firsts, &identity(&second, "video"));
        assert_eq!(colour_on(&on_firsts, LOCAL_IDENTITY), first_on_seconds);
        assert_eq!(colour_on(&on_seconds, LOCAL_IDENTITY), second_on_firsts);
        assert_ne!(first_on_seconds, second_on_firsts);
    }

    #[test]
    fn a_clash_is_settled_again_when_someone_leaves() {
        let (first, second) = clashing_people();
        let mut manager = machine(&second, &[&first]);
        let mine = colour_on(&manager, LOCAL_IDENTITY);
        assert_ne!(mine, colours::colour(&second), "first keeps the color");

        let theirs = identity(&first, "video");
        manager.remove_participant(&theirs);
        manager.set_call_people(Some(&identity(&second, "audio")), &[]);
        assert_eq!(
            colour_on(&manager, LOCAL_IDENTITY),
            colours::colour(&second)
        );
    }

    #[test]
    fn someone_joining_gets_the_colour_the_call_agreed_on() {
        let (first, second) = clashing_people();
        let mut manager = machine(&first, &[]);
        let joining = identity(&second, "video");
        manager.set_call_people(Some(&identity(&first, "audio")), &[joining.clone()]);
        manager
            .add_participant(joining.clone(), "Someone", false, DrawingMode::Any)
            .unwrap();
        assert_ne!(
            colour_on(&manager, &joining),
            colour_on(&manager, LOCAL_IDENTITY)
        );
    }

    #[test]
    fn before_the_call_is_known_people_get_their_own_colour() {
        let mut manager = ParticipantsManager::new();
        let person = identity("usr_1", "video");
        manager
            .add_participant(person.clone(), "Someone", false, DrawingMode::Any)
            .unwrap();
        assert_eq!(colour_on(&manager, &person), colours::colour("usr_1"));
    }
}
