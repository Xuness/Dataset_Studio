-- v20: count distinct observed Pins per discovery stream, including not-yet-admitted candidates.
ALTER TABLE pinterest_streams ADD COLUMN unique_pins INTEGER NOT NULL DEFAULT 0;
ALTER TABLE pinterest_streams ADD COLUMN reported_total INTEGER CHECK(reported_total>=0);
CREATE TABLE pinterest_stream_pins(scan_id TEXT NOT NULL REFERENCES pinterest_streams(scan_id),
 pin_id TEXT NOT NULL,PRIMARY KEY(scan_id,pin_id)) WITHOUT ROWID;
INSERT INTO pinterest_stream_pins
 SELECT s.scan_id,t.pin_id FROM pinterest_tasks t JOIN pinterest_streams s
 ON s.scan_id=json_extract(t.input_json,'$.scan_id') AND s.job_id=t.job_id
 WHERE t.kind='pin_admit' GROUP BY s.scan_id,t.pin_id;
UPDATE pinterest_streams SET unique_pins=(SELECT count(*) FROM pinterest_stream_pins p WHERE p.scan_id=pinterest_streams.scan_id);
CREATE TRIGGER pinterest_stream_pin_insert AFTER INSERT ON pinterest_stream_pins BEGIN
 UPDATE pinterest_streams SET unique_pins=unique_pins+1 WHERE scan_id=new.scan_id;
END;
