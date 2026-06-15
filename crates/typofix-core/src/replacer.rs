//! Будує план заміни `DeleteChars(n) + SwitchLayout(id) + TypeUnicode(text)`
//! зі збереженням регістру/Caps та апострофів. Текст іде як Unicode (не повтор
//! scancode), щоб не залежати від моменту перемикання розкладки. §3.2.

// TODO(phase-1): build_plan(buffer, target_layout) -> Vec<Action>.
