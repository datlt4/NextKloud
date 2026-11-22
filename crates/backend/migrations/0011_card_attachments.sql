-- File đính kèm của card: đường dẫn tới file thường của chủ board (vd "Deck/a.pdf").
ALTER TABLE cards ADD COLUMN attachments TEXT[] NOT NULL DEFAULT '{}';
