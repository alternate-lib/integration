CREATE SCHEMA IF NOT EXISTS alternate;

CREATE FUNCTION alternate.set_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    IF OLD IS DISTINCT FROM NEW THEN
        NEW.updated_at = now();
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;
