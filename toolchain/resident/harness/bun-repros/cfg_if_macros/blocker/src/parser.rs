pub struct Cursor {
    pub next_pos: usize,
}

impl Cursor {
    pub fn consume_ch(&mut self, input: &[u8]) -> Option<u8> {
        let ch = input.get(self.next_pos).copied();
        self.next_pos += 1;
        trace!(@chars "consume", ch);
        ch
    }

    pub fn unconsume_ch(&mut self) {
        self.next_pos -= 1;
        trace!(@chars "unconsume");
    }

    pub fn lookahead(&self, input: &[u8], offset: usize) -> Option<u8> {
        let ch = input.get(self.next_pos + offset - 1).copied();
        trace!(@chars "lookahead", ch);
        trace!(@noop);
        ch
    }
}
