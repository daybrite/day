app_title = Vitrine de Day
counter_value = { $count ->
    [one] { $count } clic
   *[other] { $count } clics
}
decrement = â
increment = +
name_placeholder = Votre nom
greeting = Bonjour, { $name } !
volume_label = Volume
progress_label = Progression
busy_label = OccupÃ©
flavor_label = Parfum
flavor_placeholder = Saisissez ou choisissez un parfum
flavor_add = Ajouter
flavor_ios_note = iOS n'a pas de contrÃ´le combo box, Day affiche donc un espace rÃ©servÃ© ici.
history_title = Historique
history_entry = le compteur est passÃ© Ã  { $value }
nav_controls = ContrÃ´les
nav_menus = Menus et dialogues
nav_text = Texte
nav_battery = Batterie
nav_sensors = Capteurs
nav_clipboard = Presse-papiers
nav_network = RÃ©seau
nav_media = MÃ©dia
nav_pickers = SÃ©lecteurs
nav_compose = Composition
nav_files = Fichiers
nav_tabs = Onglets
nav_stack = Pile
nav_list = Liste
nav_refresh = Actualiser
refresh_caption = Tirez le flux vers le bas â ou utilisez le bouton â pour recharger
refresh_status_idle = Inactif
refresh_status_refreshing = Actualisationâ¦
refresh_now = Actualiser maintenant
refresh_tier_native = Tirer pour actualiser : natif
refresh_tier_emulated = Tirer pour actualiser : Ã©mulÃ©
refresh_row = ÃlÃ©ment { $n }
nav_webview = Vue Web
nav_lottie = Lottie
nav_about = Ã propos

shapes_kinds = Types
gradients_title = DÃ©gradÃ©s
gradient_angle = Angle
shapes_transform = Transformation
shapes_angle = Angle

picker_shared_caption = Les trois styles sont liÃ©s au mÃªme signal de sÃ©lection â modifiez-en un, les autres suivent.
picker_selected = SÃ©lection
picker_segmented = SegmentÃ©
picker_menu = Menu
picker_inline = AlignÃ©

# â day-piece-datetime â
nav_dates = Date et heure
dates_caption = SÃ©lecteurs natifs de date et d'heure liÃ©s en double sens Ã  des signaux civils â les sÃ©lecteurs d'une mÃªme section partagent le mÃªme signal.
dates_date_section = Date
dates_time_section = Heure
dates_composed_section = ComposÃ©
date_compact = Compact
date_inline = Calendrier
time_compact = Compact
time_seconds = Avec les secondes
dates_composed = Date et heure
date_bounded = En 2026
date_picked = Date choisie
time_picked = Heure choisie

compose_caption = PiÃ¨ces de pure composition â sans code natif, sans fonctionnalitÃ©s cargo, sur tous les backends gratuitement.
compose_rating_label = Note en Ã©toiles
compose_rating_count = Ãtoiles sÃ©lectionnÃ©es :
compose_rating_placeholder = 1â5
compose_card_title = Surface rÃ©utilisable
compose_card_body = Marge + arriÃ¨re-plan + coins arrondis, appliquÃ©s comme Modificateur.
compose_plain_btn = Simple
compose_styled_btn = Rempli
compose_env_value = TeintÃ© par l'accent fourni
list_add = Ajouter 100
list_caption = { $count } lignes â seules les cellules visibles sont crÃ©Ã©es

webview_url_hint = Saisir une URL
webview_go = Aller
webview_back = PrÃ©cÃ©dent
webview_forward = Suivant
webview_stop = ArrÃªter
webview_reload = Recharger

lottie_caption = Une animation Lottie native, fournie en JSON (lottie-ios / lottie-android)
lottie_speed = Vitesse
stack_root_body = Une vraie pile push/pop. Son chemin est un signal de l'application.
stack_push = Empiler un dÃ©tail
stack_detail_title = Niveau { $depth }
stack_detail_body = EmpilÃ© sur le chemin. Le bouton retour natif rÃ©Ã©crit le dÃ©pilement.
stack_item_title = ÃlÃ©ment { $id }
stack_link_42 = Ouvrir item-42 avec un indice (route absolue)
stack_param_hint = Ouvert avec l'indice : {$hint}
tab_one = AperÃ§u
tab_two = DÃ©tails
tab_three = RÃ©glages
tab_one_body = L'onglet aperÃ§u. Chaque onglet conserve son propre Ã©tat.
tab_two_body = L'onglet dÃ©tails, sÃ©lectionnÃ© par sa clÃ© de route.
tab_three_body = L'onglet rÃ©glages. Les liens profonds et dayscript choisissent les onglets par clÃ©.
about_text = Une application native multiplateforme construite avec day.
modal_alert = Afficher l'alerte
modal_confirm = Confirmer
modal_delete = Supprimerâ¦
modal_sheet = Choisir un parfum
modal_prompt = Saisir le nom
alert_title = Avis
alert_body = Vos modifications ont Ã©tÃ© enregistrÃ©es.
ok = OK
confirm_title = Quitter ?
confirm_body = Voulez-vous vraiment quitter ?
delete_title = Supprimer l'Ã©lÃ©ment ?
delete_body = Cette action est irrÃ©versible.
delete = Supprimer
flavor_title = Choisissez un parfum
cancel = Annuler
vanilla = vanille
pistachio = pistache

# Files playground (docs/files.md)
files_caption = SÃ©lecteurs de fichiers natifs. Â« Ouvrir Â» lit un fichier texte dans l'Ã©diteur ; Â« Enregistrer Â» l'Ã©crit.
files_placeholder = Saisissez du texte Ã  enregistrerâ¦
files_open = Ouvrir un fichierâ¦
files_save = Enregistrer le fichierâ¦
files_opened = Ouvert : { $name }

# Battery playground (docs/battery.md)
battery_refresh = Lire la batterie
battery_level = Niveau
battery_charging = En charge
battery_reading = Batterie : { $percent } Â· { $state }
battery_reading_none = Batterie : aucune API batterie sur cette plateforme

# Aire de jeu Capteurs (docs/sensors.md)
sensors_refresh = Lire les capteurs
sensor_accelerometer = AccÃ©lÃ©romÃ¨tre
sensor_gyroscope = Gyroscope
sensor_magnetometer = MagnÃ©tomÃ¨tre
sensor_reading = x { $x } Â· y { $y } Â· z { $z } { $unit }
sensor_waiting = en attente du premier Ã©chantillonâ¦
sensor_unavailable = indisponible sur cet appareil

# Aire de jeu Presse-papiers (docs/clipboard.md)
clipboard_caption = La part day-part-clipboard lit et Ã©crit le presse-papiers systÃ¨me nativement.
clipboard_placeholder = Saisissez un texte Ã  copier
clipboard_copy = Copier
clipboard_paste = Coller
clipboard_idle = Presse-papiers intact
clipboard_copied = CopiÃ© dans le presse-papiers systÃ¨me
clipboard_copy_failed = Ãchec de la copie (pas d'API presse-papiers ici)
clipboard_pasted = CollÃ© depuis le presse-papiers systÃ¨me
clipboard_empty = Presse-papiers vide (ou illisible en arriÃ¨re-plan)

# Aire de jeu RÃ©seau (docs/network.md)
network_refresh = Lire le rÃ©seau
network_reading_online = En ligne Â· { $kind } Â· facturÃ© : { $expensive }
network_reading_offline = Hors ligne
network_reading_none = Aucune API de connectivitÃ© sur cette plateforme

# Aire de jeu MÃ©dia (docs/media.md)
media_play = Lecture
media_pause = Pause
media_load = Charger

# â Localization page (docs/localization.md) â
nav_localization = Localisation
fmt_caption = Un seul jeu de traductions â rendu conforme Ã  ICU pour chaque locale : nombres, dates, grammaire du pluriel et ordre de tri suivent la langue.
loc_locale_section = Locale en direct
loc_live_note = La locale est un signal â changer de langue re-rend chaque chaÃ®ne instantanÃ©ment. Le sens de lecture est fixÃ© au lancement (lancez en ar pour l'interface miroir).
loc_current_label = Actuelle
loc_reset = RÃ©initialiser
loc_numbers_section = Nombres
loc_dates_section = Dates et heures
loc_plurals_section = Pluriels
loc_sorting_section = Tri
fmt_number_label = GroupÃ©
fmt_fraction_label = Deux dÃ©cimales
fmt_percent_label = Pourcentage
fmt_date_label = Date longue
fmt_time_label = Heure
fmt_datetime_label = Date et heure
fmt_sorted_label = TriÃ©
fmt_number = { NUMBER($n) }
fmt_fraction = { NUMBER($n, minimumFractionDigits: 2) }
fmt_percent = { NUMBER($p, style: "percent") }
fmt_date = { DATETIME($d, dateStyle: "long") }
fmt_time = { DATETIME($t, timeStyle: "short") }
fmt_datetime = { DATETIME($dt, dateStyle: "medium", timeStyle: "short") }
plural_items = { $count ->
    [0] Rien pour l'instant
    [one] Un Ã©lÃ©ment
   *[other] { $count } Ã©lÃ©ments
}

# Aire de jeu Texte (typographie)
text_caption = Les styles sÃ©mantiques correspondent aux styles natifs et Ã  l'Ã©chelle de texte d'accessibilitÃ©.
text_styles_header = Styles
text_weights_header = Graisses
text_styling_header = Gras et italique
text_colors_header = Couleur
text_custom_header = Tailles personnalisÃ©es
text_custom_note = Font.System(pt) â mis Ã  l'Ã©chelle par la taille de texte d'accessibilitÃ© (Dynamic Type).
text_fonts_header = Polices embarquÃ©es
text_fonts_note = Font.Custom("Famille", pt) â fichiers du dossier resource/fonts/ de l'application, embarquÃ©s par day build et rÃ©solus par nom de famille sur chaque plateforme.

# Aire de jeu Menus
menus_caption = Menus natifs â la barre de menus de l'application et les menus contextuels par Ã©lÃ©ment â avec sous-menus imbriquÃ©s, raccourcis clavier et commandes d'Ã©dition standard.
menus_last = DerniÃ¨re action
menus_lifecycle = Cycle de vie
menus_target = Clic droit ici (appui long sur mobile) pour un menu contextuel
menus_shortcut_hint = Les raccourcis clavier (â/Ctrl + touche) apparaissent dans la barre de menus et fonctionnent quand l'application est active â p. ex. Nouveau (N), Enregistrer (S), Recharger (R).

# --- day-part-haptics ---
nav_haptics = Haptique
haptics_supported_yes = Moteur haptique disponible sur cette plateforme
haptics_supported_no = Aucun moteur haptique sur cette plateforme (les boutons sont silencieux)
haptics_light = LÃ©ger
haptics_medium = Moyen
haptics_heavy = Fort
haptics_success = SuccÃ¨s
haptics_warning = Avertissement
haptics_error = Erreur
haptics_selection = SÃ©lection
haptics_last = Dernier jouÃ©
haptics_none = Rien jouÃ© pour l'instant
haptics_last_played = JouÃ© : { $style }

# --- day-part-prefs ---
nav_prefs = PrÃ©fÃ©rences
prefs_caption = Conserver une chaÃ®ne entre les lancements avec day-part-prefs.
prefs_placeholder = Valeur Ã  mÃ©moriser
prefs_save = Enregistrer
prefs_load = Charger
prefs_clear = Effacer
prefs_idle = Saisissez une valeur, puis Enregistrer.
prefs_empty = (rien d'enregistrÃ©)
prefs_saved = EnregistrÃ©.
prefs_save_failed = Ãchec de l'enregistrement.
prefs_loaded = ChargÃ© depuis le stockage.
prefs_missing = Rien d'enregistrÃ© pour l'instant.
prefs_cleared = EffacÃ©.
prefs_value_label = Valeur enregistrÃ©e :

# --- bundled resources (Â§18.3) ---
nav_resources = Ressources
resources_caption = Une image chargÃ©e par nom depuis une ressource, avec accÃ¨s alÃ©atoire Ã  des donnÃ©es embarquÃ©es.
resources_numbers = numbers.bin : { $len } octets, byte[100] = { $byte }
resources_greeting = greeting.txt : { $text }

# --- day-part-deviceinfo ---
nav_deviceinfo = Appareil
deviceinfo_model = ModÃ¨le : {$value}
deviceinfo_system = SystÃ¨me : {$name} {$version}
deviceinfo_simulator = Simulateur : {$value}
deviceinfo_yes = oui
deviceinfo_no = non
deviceinfo_refresh = Actualiser

# --- day-piece-activity ---
activity_animating = Animation
activity_on = En rotation
activity_off = ArrÃªtÃ©

# --- day-piece-searchfield ---
nav_search = Recherche
search_placeholder = Rechercher un fruitâ¦
search_clear = Effacer

# --- day-piece-map ---
nav_map = Carte
map_caption = Une MKMapView native â plateformes Apple uniquement. Touchez un prÃ©rÃ©glage pour recentrer la carte en direct.
map_sf = San Francisco
map_nyc = New York

# â page tweaks (docs/tweaks.md) â
nav_tweaks = Tweaks
tweaks_intro = Les tweaks empaquetÃ©s configurent le composant natif derriÃ¨re une piÃ¨ce intÃ©grÃ©e, par toolkit. LÃ  oÃ¹ un tweak n'est pas couvert, il est sans effet â les piÃ¨ces ci-dessous restent d'origine.
tweaks_stock = D'origine
tweaks_tweaked = AjustÃ©e
tweaks_bezel_title = Biseau du bouton
tweaks_bezel_caption = day-tweak-button-bezel â AppKit uniquement : les constantes NSBezelStyle sur le vrai NSButton.
tweaks_selectable_title = LibellÃ© sÃ©lectionnable
tweaks_selectable_caption = day-tweak-label-selectable â AppKit, GTK, Android : la sÃ©lection de texte native sur un libellÃ© standard.
tweaks_selectable_text = Le texte de ce libellÃ© peut Ãªtre sÃ©lectionnÃ© et copiÃ© â essayez.
tweaks_ticks_title = Graduations du curseur
tweaks_ticks_caption = day-tweak-slider-tickmarks â AppKit, GTK, Android, Qt, WinUI, ArkUI : graduations natives, avec aimantation lÃ  oÃ¹ la plateforme la propose. Le curseur ajustÃ© s'aimante ; celui d'origine glisse.
tweaks_ref_title = VivacitÃ© du NativeRef
tweaks_ref_caption = Un NativeRef atteint le curseur ajustÃ© aprÃ¨s montage ; dÃ©montez-le et la rÃ©fÃ©rence se vide au lieu de pendre.
tweaks_ref_live = rÃ©f : vivante
tweaks_ref_cleared = rÃ©f : vidÃ©e

# â merged section pages (design overhaul) â
nav_canvas = Canevas et formes
nav_system = Appareil et capteurs
nav_services = Services systÃ¨me
controls_caption = Liaisons bidirectionnelles : chaque contrÃ´le projette un signal de l'application.
controls_basics = Essentiels
controls_feedback = Retour visuel
canvas_caption = Formes, transformations, gestes et widgets composÃ©s â tous dessinÃ©s via le canevas.
canvas_gauge = Jauge canevas
shapes_interact_hint = Glissez le curseur pour pivoter, touchez le cercle pour recolorer, dÃ©placez le carrÃ© violet.
system_caption = Les modules d'Ã©tat de l'appareil : batterie, connectivitÃ©, capteurs et identitÃ©.
services_caption = Les modules Â« agir avec l'OS Â» : presse-papiers, prÃ©fÃ©rences, haptique, fichiers et HTTP.
subscribe_label = S'abonner

# â data strings localized for the walkthrough locales (option lists, specimen rows) â
chocolate = chocolat
size_small = Petit
size_medium = Moyen
size_large = Grand
fruit_apple = Pomme
fruit_banana = Banane
fruit_cherry = Cerise
fruit_date = Datte
fruit_elderberry = Sureau
list_row = Ligne { $n }
text_style_large_title = Grand titre
text_style_title = Titre
text_style_title2 = Titre 2
text_style_title3 = Titre 3
text_style_headline = En-tÃªte
text_style_subheadline = Sous-en-tÃªte
text_style_body = Corps
text_style_callout = EncadrÃ©
text_style_footnote = Note de bas de page
text_style_caption = LÃ©gende
text_style_caption2 = LÃ©gende 2
text_weight_ultralight = Ultra-fin
text_weight_light = Fin
text_weight_regular = Normal
text_weight_medium = Moyen
text_weight_semibold = Demi-gras
text_weight_bold = Gras
text_weight_heavy = TrÃ¨s gras
text_weight_black = Noir
text_bold = Texte gras
text_italic = Texte italique
text_bolditalic = Gras italique
text_emphasis_label = Emphase
color_red = Rouge
color_green = Vert
color_blue = Bleu
color_orange = Orange

# Menus & dialogues (page fusionnÃ©e)
menus_appmenu_section = Menu de lâapplication
menus_context_section = Menu contextuel
menus_dialogs_section = Dialogues
modal_result_label = RÃ©sultat

# Page MÃ©dia
media_caption = Un lecteur multimÃ©dia natif â la vue de la plateforme, transport pilotÃ© par dÃ©clencheurs.
media_player_section = VidÃ©o

# Sections de la page Ressources
resources_image_section = Image embarquÃ©e
resources_modes_note = Une image, trois modes â Ajuster prÃ©serve les proportions, Remplir rogne, Ãtirer dÃ©forme.
image_mode_fit = Ajuster
image_mode_fill = Remplir
image_mode_stretch = Ãtirer
resources_data_section = DonnÃ©es embarquÃ©es

# Page Ã propos
about_caption = Ce quâest cette app, et la plateforme oÃ¹ elle sâexÃ©cute.
about_app_section = Cette app
about_version = Version
about_toolkit = BoÃ®te Ã  outils
about_battery = Batterie
history_hint = Touchez + ou â ci-dessus : chaque changement sâaffiche ici.

# Page Focus (docs/focus.md)
nav_focus = Focus
focus_caption = Le focus est une liaison bidirectionnelle : les changements natifs Ã©crivent le signal, et Ã©crire le signal dÃ©place le focus.
focus_group_section = Un signal, un formulaire
focus_group_caption = Trois champs liÃ©s Ã  un mÃªme signal optionnel. Cliquez ou tabulez de lâun Ã  lâautre et lâindicateur suit ; EntrÃ©e passe au champ suivant.
focus_name_label = Nom
focus_email_label = E-mail
focus_city_label = Ville
focus_current_label = Focus
focus_next = Focus suivant
focus_clear = Effacer le focus
focus_bool_section = Un contrÃ´le, un boolÃ©en
focus_bool_caption = Le mÃªme champ liÃ© Ã  un signal boolÃ©en â les boutons lâÃ©crivent ; entrer dans le champ ou en sortir lâÃ©crit en retour.
focus_bool_placeholder = Le focus arrive ici
focus_focus_btn = Donner le focus
focus_blur_btn = Retirer le focus
focus_state_label = Ãtat
focus_state_on = avec focus
focus_state_off = sans focus
focus_probe_section = Au-delÃ  des champs de texte
focus_probe_caption = Les toolkits de bureau donnent aussi le focus aux boutons, interrupteurs et curseurs ; les plateformes tactiles le rÃ©servent surtout Ã  la saisie de texte.
focus_probe_toggle = Interrupteur
focus_probe_slider = Curseur
focus_probe_button = Bouton

# HTTP fetch demo (docs/http.md) â the status readout stays raw "<status> <body>" so the
# walkthrough asserts it byte-for-byte in every locale.
http_title = HTTP
http_caption = Le module day-part-http passe par la pile HTTP de la plateforme â ses proxys, son VPN et son TLS.
http_fetch = RÃ©cupÃ©rer depuis localhost
http_idle = Rien de rÃ©cupÃ©rÃ© pour l'instant
http_tier = Pile
http_url_placeholder = https://example.com
http_check = VÃ©rifier
http_checking = VÃ©rificationâ¦
http_patch = PATCH
http_res_label = Ressource
http_res_refetch = Recharger

# Scrolling page (docs/scroll.md) â programmatic scroll targets.
nav_scrolling = DÃ©filement
scrolling_caption = DÃ©filement programmatique : un Signal amÃ¨ne la zone de dÃ©filement Ã  un bord, une position ou un Ã©lÃ©ment prÃ©cis.
scroll_to_top = Aller en haut
scroll_to_bottom = Aller en bas
scroll_to_item = Aller Ã  l'Ã©lÃ©ment 100
scrolling_item = ÃlÃ©ment { $n }

# Grid page (docs/grid.md) â grid/grid_row from basics to a stress test.
nav_grid = Grille
grid_caption = Des colonnes dimensionnÃ©es par leur contenu, des cellules qui fusionnent, et des cellules flexibles qui se partagent la largeur restante
grid_tab_basics = Bases
grid_tab_sizing = Dimensions
grid_tab_spanning = Fusion
grid_tab_composite = Composition
grid_tab_stress = Endurance
grid_basics_caption = Chaque colonne prend la largeur de sa cellule la plus large. Pas de largeurs fixes, pas d'espaceurs de remplissage.
grid_col_name = Nom
grid_col_wins = Victoires
grid_col_points = Points
grid_sizing_caption = Colonnes fixes, ajustÃ©es au contenu et flexibles dans une mÃªme grille.
grid_sizing_fixed = Fixe 80 pt
grid_sizing_content = Contenu
grid_sizing_short = Court
grid_sizing_longer = Une cellule au contenu plus long
grid_spanning_caption = Une cellule peut couvrir plusieurs colonnes ; un enfant hors de toute ligne couvre la grille entiÃ¨re.
grid_month_title = Semainier
grid_event_focus = Bloc de concentration
grid_event_review = Revue
grid_composite_caption = Formes et grille rÃ©unies : des glyphes groupÃ©s dans des colonnes au contenu, Ã  cÃ´tÃ© d'une barre de plage flexible.
grid_day_n = Jour { $n }
grid_stress_cells = { $n } lignes de 8 cellules, toutes disposÃ©es d'avance. Modifier une cellule ne remesure que celle-ci.
grid_stress_add = Ajouter 50 lignes
grid_stress_bump = IncrÃ©menter la premiÃ¨re cellule

nav_animation = Animation
anim_caption = Mettez en file d’attente échelle, rotation, opacité, décalage et teinte, puis touchez « Animer ! » pour tout animer ensemble avec la courbe et la durée choisies.

# Page Animation
anim_scale = Échelle
anim_rotation = Rotation
anim_opacity = Opacité
anim_offset_x = Décalage X
anim_offset_y = Décalage Y
anim_hue = Teinte
anim_curve = Courbe
anim_duration = Durée
anim_randomize = Aléatoire !
anim_go_label = Animer !
anim_reset_label = Réinitialiser
anim_curve_spring = Ressort
anim_curve_ease_in_out = Progressif
anim_curve_ease_out = Décéléré
anim_curve_linear = Linéaire
anim_duration_ms = { $ms } ms

# Barre de menus + menu contextuel
menu_file = Fichier
menu_new = Nouveau
menu_open = Ouvrir…
menu_open_recent = Ouvrir récent
menu_clear_menu = Effacer le menu
menu_save = Enregistrer
menu_save_as = Enregistrer sous…
menu_edit = Édition
menu_view = Affichage
menu_reload = Recharger
menu_actual_size = Taille réelle
menu_context = Contexte
menu_rename = Renommer
menu_duplicate = Dupliquer
menu_move_to = Déplacer vers
menu_inbox = Boîte de réception
menu_archive = Archiver

# Page Texte : tailles, polices embarquées, liens
text_size_pt = { $pt } pt
text_font_pacifico = Pacifico — script fluide
text_font_bungee = BUNGEE — display chromatique
text_font_specialelite = Special Elite — touches de machine à écrire
text_font_pacifico_lg = Pacifico en 36 points
text_links_section = Liens
text_links_caption = Touchez un lien pour l'ouvrir dans le navigateur du système.
text_link_icons_label = Material Symbols sur Google Fonts
text_link_mail_label = Écrire à l'équipe

# Section Fichiers : texte initial de l'éditeur
files_initial_content =
    Bonjour de Day !
    Modifiez-moi, puis Enregistrer.
