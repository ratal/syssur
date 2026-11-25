package 'System Structure' {
    import StandardViewDefinitions::*;

    part 'Doorbell System' {
        part Button;
        part Chime;
    }

    view 'Doorbell System Structure Overview' : GeneralView {
        expose 'System Structure'::**;
    }
}
