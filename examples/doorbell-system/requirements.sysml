package Requirements {
    import StandardViewDefinitions::*;
    import 'System Structure'::*;

    requirement <'CH-01'> 'Chime Tone' {
        doc /* The chime tone shall consist of one second of higher pitch bell tone, followed by one second of lower pitch bell tone, followed by one second of silence. */
    }

    requirement <'CH-02'> 'Chime Cycle' {
        doc /* The chime cycle shall consist of three chime tones. */
    }

    requirement <'CH-03'> 'Chime Cycle After Button Press' {
        doc /* When the doorbell button is pressed, the chime unit shall complete one chime cycle. */
    }

    requirement <'BT-01'> 'Flash During Chime' {
        doc /* While the chime cycle is in progress, the doorbell button shall flash. */
    }

    requirement <'BT-02'> 'Flash Cycle' {
        doc /* The button flash cycle shall be 0.5 second bright illumination followed by 0.5 second of no illumination. */
    }

    requirement <'BT-03'> 'Idle Dim Glow' {
        doc /* When the doorbell system is idle, the doorbell button shall glow dimly. */
    }

    satisfy 'Chime Cycle' by 'Doorbell System'.Chime;
    satisfy 'Idle Dim Glow' by 'Doorbell System'.Button;

    view 'Requirements Traceability' : GeneralView {
        expose Requirements::**;
        expose 'System Structure'::'Doorbell System'::Chime;
        expose 'System Structure'::'Doorbell System'::Button;
    }
}
