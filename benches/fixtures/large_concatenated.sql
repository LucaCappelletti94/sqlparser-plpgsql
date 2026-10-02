BEGIN 
    INSERT INTO parent_procedure_templates (parent_id, child_id)
    VALUES (NEW.parent_id, NEW.predecessor_id) 
    ON CONFLICT (parent_id, child_id) DO NOTHING;

    INSERT INTO parent_procedure_templates (parent_id, child_id)
    VALUES (NEW.parent_id, NEW.successor_id) 
    ON CONFLICT (parent_id, child_id) DO NOTHING;
    
    RETURN NEW;
END;
BEGIN
    INSERT INTO procedure_template_asset_models (
            name,
            procedure_template_id,
            based_on_id,
            asset_model_id
        )
    SELECT 
        pam.name,
        NEW.parent_id,
        pam.id,
        pam.asset_model_id
    FROM procedure_template_asset_models pam
    WHERE pam.procedure_template_id = NEW.child_id;
    
    RETURN NEW;
END;
BEGIN
	INSERT INTO owners (id) VALUES (NEW.id);
	RETURN NEW;
END;
BEGIN
	INSERT INTO owners (id) VALUES (NEW.id);
	RETURN NEW;
END;
BEGIN
    WITH RECURSIVE parent_groups AS (
        SELECT parent_group_id AS id FROM groups WHERE id = NEW.group_id
        UNION ALL
        SELECT g.parent_group_id FROM groups g JOIN parent_groups pg ON g.id = pg.id
    )
    INSERT INTO group_memberships (group_id, user_id)
    SELECT id, NEW.user_id FROM parent_groups WHERE id IS NOT NULL
    ON CONFLICT (group_id, user_id) DO NOTHING;
    RETURN NEW;
END;
BEGIN
    WITH RECURSIVE child_groups AS (
        SELECT id FROM groups WHERE parent_group_id = OLD.group_id
        UNION ALL
        SELECT g.id FROM groups g JOIN child_groups cg ON g.parent_group_id = cg.id
    )
    DELETE FROM group_memberships
    WHERE user_id = OLD.user_id
      AND group_id IN (SELECT id FROM child_groups);
    RETURN OLD;
END;
BEGIN
    IF NEW.event_type = 'error' THEN
        INSERT INTO audit_log (action, severity) VALUES ('error_event', 'high');
    ELSIF NEW.event_type = 'warning' THEN
        INSERT INTO audit_log (action, severity) VALUES ('warning_event', 'medium');
    ELSIF NEW.event_type = 'info' THEN
        INSERT INTO audit_log (action, severity) VALUES ('info_event', 'low');
    ELSE
        INSERT INTO audit_log (action, severity) VALUES ('unknown_event', 'unknown');
    END IF;
    RETURN NEW;
END;
BEGIN
    WITH RECURSIVE chain AS (
        SELECT id, parent_id FROM tree_nodes WHERE id = NEW.id
        UNION ALL
        SELECT c.id, t.parent_id FROM chain c JOIN tree_nodes t ON c.parent_id = t.id WHERE t.parent_id IS NOT NULL
    )
    INSERT INTO node_ancestors (node_id, ancestor_id)
    SELECT NEW.id, parent_id FROM chain WHERE parent_id IS NOT NULL;

    RETURN NEW;
END;
DECLARE
    new_id TEXT;
BEGIN
    new_id := gen_random_uuid();
    INSERT INTO todo_history (id, todo_id, action)
    VALUES (new_id, NEW.id, 'created');
    RETURN NEW;
END;